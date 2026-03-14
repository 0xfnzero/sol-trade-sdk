use crate::swqos::common::{
    default_http_client_builder, poll_transaction_confirmation, serialize_transaction_and_encode,
};
use rand::seq::IndexedRandom;
use reqwest::Client;
use std::{sync::Arc, time::Instant};

use solana_transaction_status::UiTransactionEncoding;
use std::time::Duration;

use crate::swqos::SwqosClientTrait;
use crate::swqos::{SwqosType, TradeType};
use anyhow::Result;
use solana_sdk::transaction::VersionedTransaction;

use crate::{common::SolanaRpcClient, constants::swqos::BLOCKRAZOR_TIP_ACCOUNTS};

use std::sync::atomic::{AtomicBool, Ordering};
use tokio::task::JoinHandle;

#[derive(Clone)]
pub struct BlockRazorClient {
    pub endpoint: String,
    pub auth_token: String,
    pub rpc_client: Arc<SolanaRpcClient>,
    pub http_client: Client,
    pub ping_handle: Arc<tokio::sync::Mutex<Option<JoinHandle<()>>>>,
    pub stop_ping: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl SwqosClientTrait for BlockRazorClient {
    async fn send_transaction(
        &self,
        trade_type: TradeType,
        transaction: &VersionedTransaction,
        wait_confirmation: bool,
    ) -> Result<()> {
        self.send_transaction(trade_type, transaction, wait_confirmation).await
    }

    async fn send_transactions(
        &self,
        trade_type: TradeType,
        transactions: &Vec<VersionedTransaction>,
        wait_confirmation: bool,
    ) -> Result<()> {
        self.send_transactions(trade_type, transactions, wait_confirmation).await
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
    pub fn new(rpc_url: String, endpoint: String, auth_token: String) -> Self {
        let rpc_client = SolanaRpcClient::new(rpc_url);
        // 官方文档：請求中唯一允許的 header 是 Content-Type: text/plain；避免默认 User-Agent 等导致 500
        let http_client = default_http_client_builder().user_agent("").build().unwrap();

        let client = Self {
            rpc_client: Arc::new(rpc_client),
            endpoint,
            auth_token,
            http_client,
            ping_handle: Arc::new(tokio::sync::Mutex::new(None)),
            stop_ping: Arc::new(AtomicBool::new(false)),
        };

        // Start ping task
        let client_clone = client.clone();
        tokio::spawn(async move {
            client_clone.start_ping_task().await;
        });

        client
    }

    /// Start periodic ping task to keep connections active
    async fn start_ping_task(&self) {
        let endpoint = self.endpoint.clone();
        let auth_token = self.auth_token.clone();
        let http_client = self.http_client.clone();
        let stop_ping = self.stop_ping.clone();

        let handle = tokio::spawn(async move {
            // Immediate first ping to warm connection and reduce first-submit cold start latency
            if let Err(e) = Self::send_ping_request(&http_client, &endpoint, &auth_token).await {
                if crate::common::sdk_log::sdk_log_enabled() {
                    eprintln!("BlockRazor ping request failed: {}", e);
                }
            }
            let mut interval = tokio::time::interval(Duration::from_secs(30)); // 30s keepalive to avoid server ~5min idle close
            loop {
                interval.tick().await;
                if stop_ping.load(Ordering::Relaxed) {
                    break;
                }
                if let Err(e) = Self::send_ping_request(&http_client, &endpoint, &auth_token).await
                {
                    if crate::common::sdk_log::sdk_log_enabled() {
                        eprintln!("BlockRazor ping request failed: {}", e);
                    }
                }
            }
        });

        // Update ping_handle - use Mutex to safely update
        {
            let mut ping_guard = self.ping_handle.lock().await;
            if let Some(old_handle) = ping_guard.as_ref() {
                old_handle.abort();
            }
            *ping_guard = Some(handle);
        }
    }

    /// Send ping request: POST /v2/health?auth=... (Keep Alive). Only required param: auth.
    async fn send_ping_request(
        http_client: &Client,
        endpoint: &str,
        auth_token: &str,
    ) -> Result<()> {
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
            eprintln!("BlockRazor ping request failed with status: {}", status);
        }
        Ok(())
    }

    /// Send transaction via v2 API: plain Base64 body, Content-Type: text/plain. Only required URI param: auth.
    /// 文档要求：auth 以 URI 参数传入；body 为纯 Base64 编码交易；唯一允许的 header 为 Content-Type: text/plain。
    pub async fn send_transaction(
        &self,
        trade_type: TradeType,
        transaction: &VersionedTransaction,
        wait_confirmation: bool,
    ) -> Result<()> {
        let start_time = Instant::now();
        let (content, signature) =
            serialize_transaction_and_encode(transaction, UiTransactionEncoding::Base64)?;

        let response = self
            .http_client
            .post(&self.endpoint)
            .query(&[("auth", self.auth_token.as_str())])
            .header("Content-Type", "text/plain")
            .body(content)
            .send()
            .await?;

        let status = response.status();
        if status.is_success() {
            let _ = response.bytes().await;
            if crate::common::sdk_log::sdk_log_enabled() {
                println!(" [blockrazor] {} submitted: {:?}", trade_type, start_time.elapsed());
            }
        } else {
            let body = response.text().await.unwrap_or_default();
            if crate::common::sdk_log::sdk_log_enabled() {
                eprintln!(
                    " [blockrazor] {} submission failed: status {} body: {}",
                    trade_type, status, body
                );
            }
            return Err(anyhow::anyhow!(
                "BlockRazor sendTransaction failed: status {} body: {}",
                status,
                body
            ));
        }

        let start_time = Instant::now();
        match poll_transaction_confirmation(&self.rpc_client, signature, wait_confirmation).await {
            Ok(_) => (),
            Err(e) => {
                if crate::common::sdk_log::sdk_log_enabled() {
                    println!(" signature: {:?}", signature);
                    println!(
                        " [blockrazor] {} confirmation failed: {:?}",
                        trade_type,
                        start_time.elapsed()
                    );
                }
                return Err(e);
            }
        }
        if wait_confirmation && crate::common::sdk_log::sdk_log_enabled() {
            println!(" signature: {:?}", signature);
            println!(" [blockrazor] {} confirmed: {:?}", trade_type, start_time.elapsed());
        }

        Ok(())
    }

    pub async fn send_transactions(
        &self,
        trade_type: TradeType,
        transactions: &Vec<VersionedTransaction>,
        wait_confirmation: bool,
    ) -> Result<()> {
        for transaction in transactions {
            self.send_transaction(trade_type, transaction, wait_confirmation).await?;
        }
        Ok(())
    }
}

impl Drop for BlockRazorClient {
    fn drop(&mut self) {
        // Ensure ping task stops when client is destroyed
        self.stop_ping.store(true, Ordering::Relaxed);

        // Try to stop ping task immediately
        // Use tokio::spawn to avoid blocking Drop
        let ping_handle = self.ping_handle.clone();
        tokio::spawn(async move {
            let mut ping_guard = ping_handle.lock().await;
            if let Some(handle) = ping_guard.as_ref() {
                handle.abort();
            }
            *ping_guard = None;
        });
    }
}
