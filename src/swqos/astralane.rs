use crate::swqos::common::{default_http_client_builder, poll_transaction_confirmation};
use rand::seq::IndexedRandom;
use reqwest::Client;
use std::{sync::Arc, time::Instant};
use tracing::{error, info, warn};

use std::time::Duration;
use anyhow::Result;
use bincode::serialize as bincode_serialize;
use solana_client::rpc_client::SerializableTransaction;
use solana_sdk::transaction::VersionedTransaction;
use crate::swqos::{SwqosType, TradeType};
use crate::swqos::SwqosClientTrait;

use crate::{common::SolanaRpcClient, constants::swqos::ASTRALANE_TIP_ACCOUNTS};

use tokio::task::JoinHandle;
use std::sync::atomic::{AtomicBool, Ordering};

/// Empty body for getHealth POST; avoid per-request allocation.
static PING_BODY: &[u8] = &[];

use crate::swqos::astralane_quic::AstralaneQuicClient;

#[derive(Clone)]
pub enum AstralaneBackend {
    Http {
        endpoint: String,
        auth_token: String,
        http_client: Client,
        ping_handle: Arc<tokio::sync::Mutex<Option<JoinHandle<()>>>>,
        stop_ping: Arc<AtomicBool>,
    },
    Quic(Arc<AstralaneQuicClient>),
}

#[derive(Clone)]
pub struct AstralaneClient {
    pub rpc_client: Arc<SolanaRpcClient>,
    backend: AstralaneBackend,
}

#[async_trait::async_trait]
impl SwqosClientTrait for AstralaneClient {
    async fn send_transaction(&self, trade_type: TradeType, transaction: &VersionedTransaction, wait_confirmation: bool) -> Result<()> {
        self.send_transaction_impl(trade_type, transaction, wait_confirmation).await
    }

    async fn send_transactions(&self, trade_type: TradeType, transactions: &Vec<VersionedTransaction>, wait_confirmation: bool) -> Result<()> {
        for transaction in transactions {
            self.send_transaction_impl(trade_type, transaction, wait_confirmation).await?;
        }
        Ok(())
    }

    fn get_tip_account(&self) -> Result<String> {
        let tip_account = *ASTRALANE_TIP_ACCOUNTS.choose(&mut rand::rng()).or_else(|| ASTRALANE_TIP_ACCOUNTS.first()).unwrap();
        Ok(tip_account.to_string())
    }

    fn get_swqos_type(&self) -> SwqosType {
        SwqosType::Astralane
    }
}

impl AstralaneClient {
    /// 使用 HTTP（irisb）提交。
    pub fn new(rpc_url: String, endpoint: String, auth_token: String) -> Self {
        let rpc_client = SolanaRpcClient::new(rpc_url);
        let http_client = default_http_client_builder().build().unwrap();
        let ping_handle = Arc::new(tokio::sync::Mutex::new(None));
        let stop_ping = Arc::new(AtomicBool::new(false));

        let client = Self {
            rpc_client: Arc::new(rpc_client),
            backend: AstralaneBackend::Http {
                endpoint,
                auth_token,
                http_client,
                ping_handle,
                stop_ping,
            },
        };
        let client_clone = client.clone();
        tokio::spawn(async move {
            client_clone.start_ping_task().await;
        });
        client
    }

    /// 使用 QUIC 提交。
    pub async fn new_quic(rpc_url: String, quic_endpoint: &str, api_key: String) -> Result<Self> {
        let rpc_client = SolanaRpcClient::new(rpc_url);
        let quic_client = AstralaneQuicClient::connect(quic_endpoint, &api_key).await?;
        Ok(Self {
            rpc_client: Arc::new(rpc_client),
            backend: AstralaneBackend::Quic(Arc::new(quic_client)),
        })
    }

    async fn start_ping_task(&self) {
        match &self.backend {
            AstralaneBackend::Http { endpoint, auth_token, http_client, ping_handle, stop_ping } => {
            let endpoint = endpoint.clone();
            let auth_token = auth_token.clone();
            let http_client = http_client.clone();
            let ping_handle = ping_handle.clone();
            let stop_ping = stop_ping.clone();
            let handle = tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(30));
                loop {
                    interval.tick().await;
                    if stop_ping.load(Ordering::Relaxed) {
                        break;
                    }
                    if let Err(e) = Self::send_ping_request(&http_client, &endpoint, &auth_token).await {
                        warn!(target: "sol_trade_sdk", "Astralane ping request failed: {}", e);
                    }
                }
            });
            let mut guard = ping_handle.lock().await;
            if let Some(old) = guard.as_ref() {
                old.abort();
            }
            *guard = Some(handle);
            }
            AstralaneBackend::Quic(_) => {}
        }
    }

    /// Send ping request: POST endpoint?api-key=...&method=getHealth
    async fn send_ping_request(http_client: &Client, endpoint: &str, auth_token: &str) -> Result<()> {
        let response = http_client
            .post(endpoint)
            .query(&[("api-key", auth_token), ("method", "getHealth")])
            .timeout(Duration::from_millis(1500))
            .body(PING_BODY)
            .send()
            .await?;
        let status = response.status();
        let _ = response.bytes().await;
        if !status.is_success() {
            warn!(target: "sol_trade_sdk", "Astralane ping request returned non-success status: {}", status);
        }
        Ok(())
    }

    async fn send_transaction_impl(&self, trade_type: TradeType, transaction: &VersionedTransaction, wait_confirmation: bool) -> Result<()> {
        let start_time = Instant::now();
        let signature = transaction.get_signature();
        let body_bytes = bincode_serialize(transaction).map_err(|e| anyhow::anyhow!("Astralane binary serialize failed: {}", e))?;

        match &self.backend {
            AstralaneBackend::Http { endpoint, auth_token, http_client, .. } => {
                let response = http_client
                    .post(endpoint)
                    .query(&[("api-key", auth_token.as_str()), ("method", "sendTransaction")])
                    .header("Content-Type", "application/octet-stream")
                    .body(body_bytes)
                    .send()
                    .await?;
                let status = response.status();
                let _ = response.bytes().await;
                if status.is_success() {
                    info!(target: "sol_trade_sdk", "[astralane] {} submitted: {:?}", trade_type, start_time.elapsed());
                } else {
                    error!(target: "sol_trade_sdk", "[astralane] {} submission failed: status {}", trade_type, status);
                    return Err(anyhow::anyhow!("Astralane sendTransaction failed: {}", status));
                }
            }
            AstralaneBackend::Quic(quic) => {
                quic.send_transaction(&body_bytes).await?;
                info!(target: "sol_trade_sdk", "[astralane-quic] {} submitted: {:?}", trade_type, start_time.elapsed());
            }
        }

        let start_time = Instant::now();
        match poll_transaction_confirmation(&self.rpc_client, *signature, wait_confirmation).await {
            Ok(_) => (),
            Err(e) => {
                info!(target: "sol_trade_sdk", "signature: {:?}", signature);
                error!(target: "sol_trade_sdk", "[astralane] {} confirmation failed: {:?}", trade_type, start_time.elapsed());
                return Err(e);
            }
        }
        if wait_confirmation {
            info!(target: "sol_trade_sdk", "signature: {:?}", signature);
            info!(target: "sol_trade_sdk", "[astralane] {} confirmed: {:?}", trade_type, start_time.elapsed());
        }
        Ok(())
    }
}

impl Drop for AstralaneClient {
    fn drop(&mut self) {
        match &self.backend {
            AstralaneBackend::Http { stop_ping, ping_handle, .. } => {
                stop_ping.store(true, Ordering::Relaxed);
                let ping_handle = ping_handle.clone();
                tokio::spawn(async move {
                    let mut guard = ping_handle.lock().await;
                    if let Some(handle) = guard.as_ref() {
                        handle.abort();
                    }
                    *guard = None;
                });
            }
            AstralaneBackend::Quic(_) => {}
        }
    }
}
