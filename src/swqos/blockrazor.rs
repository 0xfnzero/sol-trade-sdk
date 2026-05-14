use crate::swqos::common::{
    default_http_client_builder, poll_transaction_confirmation, serialize_transaction_and_encode,
};
use rand::seq::IndexedRandom;
use reqwest::Client;
use std::{sync::Arc, time::Instant};

use arc_swap::ArcSwap;
use solana_transaction_status::UiTransactionEncoding;
use std::time::Duration;

use crate::swqos::SwqosClientTrait;
use crate::swqos::{SwqosType, TradeType};
use anyhow::Result;
use solana_sdk::transaction::VersionedTransaction;

use crate::{common::SolanaRpcClient, constants::swqos::BLOCKRAZOR_TIP_ACCOUNTS};

use std::sync::atomic::{AtomicBool, Ordering};
use tokio::task::JoinHandle;
use tonic::metadata::AsciiMetadataValue;
use tonic::transport::Channel;

// Include pre-generated gRPC code
pub mod serverpb {
    include!("pb/serverpb.rs");
}

// gRPC client wrapper
#[derive(Clone)]
pub struct BlockRazorGrpcClient {
    channel: Channel,
    auth_token: String,
}

impl BlockRazorGrpcClient {
    pub fn new(channel: Channel, auth_token: String) -> Self {
        Self { channel, auth_token }
    }

    pub async fn get_health(&self) -> Result<String> {
        let mut client = serverpb::server_client::ServerClient::new(self.channel.clone());
        let apikey = AsciiMetadataValue::try_from(self.auth_token.as_str())
            .map_err(|e| anyhow::anyhow!("Invalid API key format: {}", e))?;

        let mut request = tonic::Request::new(serverpb::HealthRequest {});
        request.metadata_mut().insert("apikey", apikey);

        let response = client
            .get_health(request)
            .await
            .map_err(|e| anyhow::anyhow!("gRPC health check failed: {}", e))?;
        Ok(response.into_inner().status)
    }

    pub async fn send_transaction(
        &self,
        transaction: String,
        mode: String,
        safe_window: Option<i32>,
        revert_protection: bool,
    ) -> Result<String> {
        // 检查交易数据大小
        if crate::common::sdk_log::sdk_log_enabled() {
            eprintln!("BlockRazor transaction size: {} bytes", transaction.len());
        }

        let mut client = serverpb::server_client::ServerClient::new(self.channel.clone());
        let apikey = AsciiMetadataValue::try_from(self.auth_token.as_str())
            .map_err(|e| anyhow::anyhow!("Invalid API key format: {}", e))?;

        let mut request = tonic::Request::new(serverpb::SendRequest {
            transaction,
            mode: String::from(mode),
            safe_window,
            revert_protection,
        });
        request.metadata_mut().insert("apikey", apikey);

        let response = client
            .send_transaction(request)
            .await
            .map_err(|e| anyhow::anyhow!("gRPC send transaction failed: {}", e))?;
        Ok(response.into_inner().signature)
    }
}

#[derive(Clone)]
pub enum BlockRazorBackend {
    Grpc {
        endpoint: String,
        auth_token: String,
        grpc_client: Arc<ArcSwap<BlockRazorGrpcClient>>,
        ping_handle: Arc<tokio::sync::Mutex<Option<JoinHandle<()>>>>,
        stop_ping: Arc<AtomicBool>,
        /// When true, gRPC send_transaction sets revert_protection=true for MEV protection.
        mev_protection: bool,
    },
    Http {
        endpoint: String,
        auth_token: String,
        http_client: Client,
        ping_handle: Arc<tokio::sync::Mutex<Option<JoinHandle<()>>>>,
        stop_ping: Arc<AtomicBool>,
        /// When true, HTTP request adds revertProtection=true query param for MEV protection.
        mev_protection: bool,
    },
}

#[derive(Clone)]
pub struct BlockRazorClient {
    pub rpc_client: Arc<SolanaRpcClient>,
    backend: BlockRazorBackend,
}

#[async_trait::async_trait]
impl SwqosClientTrait for BlockRazorClient {
    async fn send_transaction(
        &self,
        trade_type: TradeType,
        transaction: &VersionedTransaction,
        wait_confirmation: bool,
    ) -> Result<()> {
        self.send_transaction_impl(trade_type, transaction, wait_confirmation).await
    }

    async fn send_transactions(
        &self,
        trade_type: TradeType,
        transactions: &Vec<VersionedTransaction>,
        wait_confirmation: bool,
    ) -> Result<()> {
        for transaction in transactions {
            self.send_transaction_impl(trade_type, transaction, wait_confirmation).await?;
        }
        Ok(())
    }

    fn get_tip_account(&self) -> Result<String> {
        let tip_account = *BLOCKRAZOR_TIP_ACCOUNTS
            .choose(&mut rand::rng())
            .or_else(|| BLOCKRAZOR_TIP_ACCOUNTS.first())
            .unwrap();
        Ok(tip_account.to_string())
    }

    fn get_swqos_type(&self) -> SwqosType {
        SwqosType::BlockRazor
    }
}

impl BlockRazorClient {
    pub async fn new(rpc_url: String, endpoint: String, auth_token: String) -> Result<Self> {
        // 默认使用 HTTP 模式，避免 gRPC FRAME_SIZE_ERROR
        Ok(Self::new_http(rpc_url, endpoint, auth_token, false))
    }

    pub async fn new_grpc(
        rpc_url: String,
        endpoint: String,
        auth_token: String,
        mev_protection: bool,
    ) -> Result<Self> {
        let rpc_client = SolanaRpcClient::new(rpc_url);

        // 配置 Channel，增加连接超时
        let channel = tonic::transport::Channel::from_shared(endpoint.clone())
            .map_err(|e| anyhow::anyhow!("Invalid gRPC endpoint: {}", e))?
            .timeout(Duration::from_secs(30))
            .connect()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to connect to gRPC endpoint: {}", e))?;

        let grpc_client =
            Arc::new(ArcSwap::from_pointee(BlockRazorGrpcClient::new(channel, auth_token.clone())));
        let ping_handle = Arc::new(tokio::sync::Mutex::new(None));
        let stop_ping = Arc::new(AtomicBool::new(false));

        let client = Self {
            rpc_client: Arc::new(rpc_client),
            backend: BlockRazorBackend::Grpc {
                endpoint,
                auth_token,
                grpc_client,
                ping_handle,
                stop_ping,
                mev_protection,
            },
        };

        let client_clone = client.clone();
        tokio::spawn(async move {
            client_clone.start_ping_task().await;
        });

        Ok(client)
    }

    pub fn new_http(
        rpc_url: String,
        endpoint: String,
        auth_token: String,
        mev_protection: bool,
    ) -> Self {
        let rpc_client = SolanaRpcClient::new(rpc_url);
        let http_client = default_http_client_builder().user_agent("").build().unwrap();
        let ping_handle = Arc::new(tokio::sync::Mutex::new(None));
        let stop_ping = Arc::new(AtomicBool::new(false));

        let client = Self {
            rpc_client: Arc::new(rpc_client),
            backend: BlockRazorBackend::Http {
                endpoint,
                auth_token,
                http_client,
                ping_handle,
                stop_ping,
                mev_protection,
            },
        };

        let client_clone = client.clone();
        tokio::spawn(async move {
            client_clone.start_ping_task().await;
        });

        client
    }

    async fn start_ping_task(&self) {
        match &self.backend {
            BlockRazorBackend::Grpc {
                grpc_client,
                ping_handle,
                stop_ping,
                endpoint,
                auth_token,
                ..
            } => {
                let grpc_client = grpc_client.clone();
                let ping_handle = ping_handle.clone();
                let stop_ping = stop_ping.clone();
                let endpoint = endpoint.clone();
                let auth_token = auth_token.clone();

                let handle = tokio::spawn(async move {
                    let mut delay = 1u64;

                    // 初始健康检查
                    {
                        let client = grpc_client.load();
                        if let Err(e) = client.get_health().await {
                            if crate::common::sdk_log::sdk_log_enabled() {
                                eprintln!("BlockRazor gRPC initial health check failed: {}", e);
                            }
                        }
                    }

                    let mut interval = tokio::time::interval(Duration::from_secs(30));
                    loop {
                        interval.tick().await;

                        if stop_ping.load(Ordering::Relaxed) {
                            break;
                        }

                        // 健康检查（使用 load() 无锁读取）
                        let client = grpc_client.load();
                        match client.get_health().await {
                            Ok(_) => {
                                delay = 1; // 成功，重置延迟
                            }
                            Err(e) => {
                                if crate::common::sdk_log::sdk_log_enabled() {
                                    eprintln!("BlockRazor gRPC health check failed: {} - reconnecting in {}s", e, delay);
                                }

                                // 等待指数退避时间
                                tokio::time::sleep(Duration::from_secs(delay)).await;
                                delay = (delay * 2).min(60);

                                // 尝试重连
                                match Self::reconnect_grpc(&endpoint, &auth_token).await {
                                    Ok(new_client) => {
                                        // 使用 swap() 无锁替换客户端
                                        grpc_client.swap(Arc::new(new_client));
                                        delay = 1; // 重置延迟
                                        if crate::common::sdk_log::sdk_log_enabled() {
                                            eprintln!("BlockRazor gRPC reconnected successfully");
                                        }
                                    }
                                    Err(reconnect_err) => {
                                        if crate::common::sdk_log::sdk_log_enabled() {
                                            eprintln!(
                                                "BlockRazor gRPC reconnect failed: {}",
                                                reconnect_err
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                });

                let mut ping_guard = ping_handle.lock().await;
                if let Some(old_handle) = ping_guard.as_ref() {
                    old_handle.abort();
                }
                *ping_guard = Some(handle);
            }
            BlockRazorBackend::Http {
                endpoint,
                auth_token,
                http_client,
                ping_handle,
                stop_ping,
                ..
            } => {
                let endpoint = endpoint.clone();
                let auth_token = auth_token.clone();
                let http_client = http_client.clone();
                let ping_handle = ping_handle.clone();
                let stop_ping = stop_ping.clone();

                let handle = tokio::spawn(async move {
                    if let Err(e) = Self::send_http_ping(&http_client, &endpoint, &auth_token).await
                    {
                        if crate::common::sdk_log::sdk_log_enabled() {
                            eprintln!("BlockRazor HTTP ping request failed: {}", e);
                        }
                    }
                    let mut interval = tokio::time::interval(Duration::from_secs(30));
                    loop {
                        interval.tick().await;
                        if stop_ping.load(Ordering::Relaxed) {
                            break;
                        }
                        if let Err(e) =
                            Self::send_http_ping(&http_client, &endpoint, &auth_token).await
                        {
                            if crate::common::sdk_log::sdk_log_enabled() {
                                eprintln!("BlockRazor HTTP ping request failed: {}", e);
                            }
                        }
                    }
                });

                let mut ping_guard = ping_handle.lock().await;
                if let Some(old_handle) = ping_guard.as_ref() {
                    old_handle.abort();
                }
                *ping_guard = Some(handle);
            }
        }
    }

    async fn send_http_ping(http_client: &Client, endpoint: &str, auth_token: &str) -> Result<()> {
        let ping_url = endpoint.replace("/v2/sendTransaction", "/v2/health");
        let response = http_client
            .post(&ping_url)
            .query(&[("auth", auth_token)])
            .header("Content-Type", "text/plain")
            .timeout(Duration::from_millis(1500))
            .body(&[] as &[u8])
            .send()
            .await?;
        let status = response.status();
        let _ = response.bytes().await;
        if !status.is_success() {
            eprintln!("BlockRazor HTTP ping request failed with status: {}", status);
        }
        Ok(())
    }

    /// 重新建立 gRPC 连接
    async fn reconnect_grpc(endpoint: &str, auth_token: &str) -> Result<BlockRazorGrpcClient> {
        let channel = tonic::transport::Channel::from_shared(endpoint.to_string())
            .map_err(|e| anyhow::anyhow!("Invalid gRPC endpoint: {}", e))?
            .timeout(Duration::from_secs(30))
            .connect()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to reconnect to gRPC endpoint: {}", e))?;

        Ok(BlockRazorGrpcClient::new(channel, auth_token.to_string()))
    }

    async fn send_transaction_impl(
        &self,
        trade_type: TradeType,
        transaction: &VersionedTransaction,
        wait_confirmation: bool,
    ) -> Result<()> {
        let start_time = Instant::now();

        match &self.backend {
            BlockRazorBackend::Grpc { grpc_client, mev_protection, .. } => {
                let (content, _signature) =
                    serialize_transaction_and_encode(transaction, UiTransactionEncoding::Base64)?;

                // 使用 load() 无锁获取客户端引用
                let client = grpc_client.load();
                let signature = client
                    .send_transaction(
                        content,
                        // mev_protection=true: sandwichMitigation mode skips blacklisted Leader slots (MEV protection).
                        // revert_protection is unrelated to MEV; keep false.
                        if *mev_protection {
                            "sandwichMitigation".to_string()
                        } else {
                            "fast".to_string()
                        },
                        None,
                        false,
                    )
                    .await;
                match signature {
                    Ok(sig) => {
                        if !sig.is_empty() {
                            if crate::common::sdk_log::sdk_log_enabled() {
                                crate::common::sdk_log::log_swqos_submitted(
                                    "BlockRazor",
                                    trade_type,
                                    start_time.elapsed(),
                                );
                            }
                        } else {
                            if crate::common::sdk_log::sdk_log_enabled() {
                                crate::common::sdk_log::log_swqos_submission_failed(
                                    "BlockRazor",
                                    trade_type,
                                    start_time.elapsed(),
                                    "empty signature".to_string(),
                                );
                            }
                            return Err(anyhow::anyhow!(
                                "BlockRazor gRPC returned empty signature"
                            ));
                        }
                    }
                    Err(e) => {
                        if crate::common::sdk_log::sdk_log_enabled() {
                            crate::common::sdk_log::log_swqos_submission_failed(
                                "BlockRazor",
                                trade_type,
                                start_time.elapsed(),
                                format!("gRPC error: {}", e),
                            );
                        }
                        return Err(anyhow::anyhow!(
                            "BlockRazor gRPC sendTransaction failed: {}",
                            e
                        ));
                    }
                }
            }
            BlockRazorBackend::Http {
                endpoint, auth_token, http_client, mev_protection, ..
            } => {
                let (content, _signature) =
                    serialize_transaction_and_encode(transaction, UiTransactionEncoding::Base64)?;

                let query_params: Vec<(&str, &str)> = vec![
                    ("auth", auth_token.as_str()),
                    // mev_protection=true: sandwichMitigation mode skips blacklisted Leader slots (MEV protection).
                    // revertProtection is unrelated to MEV; not set.
                    ("mode", if *mev_protection { "sandwichMitigation" } else { "fast" }),
                ];

                let response = http_client
                    .post(endpoint)
                    .query(&query_params)
                    .header("Content-Type", "text/plain")
                    .body(content)
                    .send()
                    .await?;

                let status = response.status();
                if status.is_success() {
                    let _ = response.bytes().await;
                    if crate::common::sdk_log::sdk_log_enabled() {
                        crate::common::sdk_log::log_swqos_submitted(
                            "blockrazor",
                            trade_type,
                            start_time.elapsed(),
                        );
                    }
                } else {
                    let body = response.text().await.unwrap_or_default();
                    if crate::common::sdk_log::sdk_log_enabled() {
                        crate::common::sdk_log::log_swqos_submission_failed(
                            "blockrazor",
                            trade_type,
                            start_time.elapsed(),
                            format!("status {} body: {}", status, body),
                        );
                    }
                    return Err(anyhow::anyhow!(
                        "BlockRazor HTTP sendTransaction failed: status {} body: {}",
                        status,
                        body
                    ));
                }
            }
        }

        let start_time = Instant::now();
        let signature = transaction.signatures[0];

        match poll_transaction_confirmation(&self.rpc_client, signature, wait_confirmation).await {
            Ok(_) => (),
            Err(e) => {
                if crate::common::sdk_log::sdk_log_enabled() {
                    println!(" signature: {:?}", signature);
                    println!(
                        " [{:width$}] {} confirmation failed: {:?}",
                        "blockrazor",
                        trade_type,
                        start_time.elapsed(),
                        width = crate::common::sdk_log::SWQOS_LABEL_WIDTH
                    );
                }
                return Err(e);
            }
        }
        if wait_confirmation && crate::common::sdk_log::sdk_log_enabled() {
            println!(" signature: {:?}", signature);
            println!(
                " [{:width$}] {} confirmed: {:?}",
                "blockrazor",
                trade_type,
                start_time.elapsed(),
                width = crate::common::sdk_log::SWQOS_LABEL_WIDTH
            );
        }

        Ok(())
    }
}

impl Drop for BlockRazorClient {
    fn drop(&mut self) {
        match &self.backend {
            BlockRazorBackend::Grpc { stop_ping, ping_handle, .. }
            | BlockRazorBackend::Http { stop_ping, ping_handle, .. } => {
                stop_ping.store(true, Ordering::Relaxed);

                let ping_handle = ping_handle.clone();
                tokio::spawn(async move {
                    let mut ping_guard = ping_handle.lock().await;
                    if let Some(handle) = ping_guard.as_ref() {
                        handle.abort();
                    }
                    *ping_guard = None;
                });
            }
        }
    }
}
