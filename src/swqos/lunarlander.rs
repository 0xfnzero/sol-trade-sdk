use crate::swqos::common::{default_http_client_builder, poll_transaction_confirmation};
use rand::seq::IndexedRandom;
use reqwest::Client;
use std::{sync::Arc, time::Instant};

use crate::swqos::SwqosClientTrait;
use crate::swqos::{SwqosType, TradeType};
use anyhow::Result;
use bincode::serialize as bincode_serialize;
use solana_sdk::transaction::VersionedTransaction;
use std::time::Duration;

use crate::{common::SolanaRpcClient, constants::swqos::LUNARLANDER_TIP_ACCOUNTS};

use std::sync::atomic::{AtomicBool, Ordering};
use tokio::task::JoinHandle;

use lunar_lander_quic_client::{ClientOptions, LunarLanderQuicClient};

#[derive(Clone)]
pub enum LunarLanderBackend {
    Http {
        endpoint: String,
        auth_token: String,
        http_client: Client,
        ping_handle: Arc<tokio::sync::Mutex<Option<JoinHandle<()>>>>,
        stop_ping: Arc<AtomicBool>,
    },
    Quic(Arc<LunarLanderQuicClient>),
}

#[derive(Clone)]
pub struct LunarLanderClient {
    pub rpc_client: Arc<SolanaRpcClient>,
    backend: LunarLanderBackend,
}

#[async_trait::async_trait]
impl SwqosClientTrait for LunarLanderClient {
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
        let tip_account = *LUNARLANDER_TIP_ACCOUNTS
            .choose(&mut rand::rng())
            .or_else(|| LUNARLANDER_TIP_ACCOUNTS.first())
            .unwrap();
        Ok(tip_account.to_string())
    }

    fn get_swqos_type(&self) -> SwqosType {
        SwqosType::LunarLander
    }
}

impl LunarLanderClient {
    /// Create an HTTP binary client (POST /send-bin with bincode body).
    pub fn new(rpc_url: String, endpoint: String, auth_token: String) -> Self {
        let rpc_client = SolanaRpcClient::new(rpc_url);
        let http_client = default_http_client_builder().build().unwrap();
        let ping_handle = Arc::new(tokio::sync::Mutex::new(None));
        let stop_ping = Arc::new(AtomicBool::new(false));

        let client = Self {
            rpc_client: Arc::new(rpc_client),
            backend: LunarLanderBackend::Http {
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

    /// Create a QUIC client (port 16888, cert CN = api_key, fire-and-forget unidirectional streams).
    pub async fn new_quic(
        rpc_url: String,
        quic_endpoint: &str,
        api_key: String,
        mev_protection: bool,
    ) -> Result<Self> {
        let rpc_client = SolanaRpcClient::new(rpc_url);
        let quic_client = LunarLanderQuicClient::connect_with_options(
            quic_endpoint,
            api_key,
            quic_client_options(mev_protection),
        )
        .await?;
        Ok(Self {
            rpc_client: Arc::new(rpc_client),
            backend: LunarLanderBackend::Quic(Arc::new(quic_client)),
        })
    }

    async fn start_ping_task(&self) {
        match &self.backend {
            LunarLanderBackend::Http {
                endpoint,
                auth_token,
                http_client,
                ping_handle,
                stop_ping,
            } => {
                let endpoint = endpoint.clone();
                let auth_token = auth_token.clone();
                let http_client = http_client.clone();
                let ping_handle = ping_handle.clone();
                let stop_ping = stop_ping.clone();
                let handle = tokio::spawn(async move {
                    // Immediate first ping to warm connection.
                    let _ = Self::send_ping_request(&http_client, &endpoint, &auth_token).await;
                    let mut interval = tokio::time::interval(Duration::from_secs(30));
                    loop {
                        interval.tick().await;
                        if stop_ping.load(Ordering::Relaxed) {
                            break;
                        }
                        if let Err(e) =
                            Self::send_ping_request(&http_client, &endpoint, &auth_token).await
                        {
                            if crate::common::sdk_log::sdk_log_enabled() {
                                eprintln!("LunarLander ping request failed: {}", e);
                            }
                        }
                    }
                });
                let mut guard = ping_handle.lock().await;
                if let Some(old) = guard.as_ref() {
                    old.abort();
                }
                *guard = Some(handle);
            }
            LunarLanderBackend::Quic(_) => {}
        }
    }

    /// GET {endpoint}/ping for HTTP keepalive.
    async fn send_ping_request(
        http_client: &Client,
        endpoint: &str,
        auth_token: &str,
    ) -> Result<()> {
        let url = format!("{}/ping", endpoint.trim_end_matches('/'));
        let response = http_client
            .get(&url)
            .header("x-api-key", auth_token)
            .timeout(Duration::from_millis(1500))
            .send()
            .await?;
        let _ = response.bytes().await;
        Ok(())
    }

    async fn send_transaction_impl(
        &self,
        trade_type: TradeType,
        transaction: &VersionedTransaction,
        wait_confirmation: bool,
    ) -> Result<()> {
        let start_time = Instant::now();
        let signature = *transaction
            .signatures
            .first()
            .ok_or_else(|| anyhow::anyhow!("LunarLander transaction has no signature"))?;
        let body_bytes = bincode_serialize(transaction)
            .map_err(|e| anyhow::anyhow!("LunarLander binary serialize failed: {}", e))?;

        match &self.backend {
            LunarLanderBackend::Http { endpoint, auth_token, http_client, .. } => {
                let url = format!("{}/send-bin", endpoint.trim_end_matches('/'));
                let response = http_client
                    .post(&url)
                    .header("x-api-key", auth_token)
                    .header("Content-Type", "application/octet-stream")
                    .body(body_bytes)
                    .send()
                    .await?;
                let status = response.status();
                let _ = response.bytes().await;
                if status.is_success() {
                    if crate::common::sdk_log::sdk_log_enabled() {
                        crate::common::sdk_log::log_swqos_submitted(
                            "LunarLander",
                            trade_type,
                            start_time.elapsed(),
                        );
                    }
                } else {
                    if crate::common::sdk_log::sdk_log_enabled() {
                        crate::common::sdk_log::log_swqos_submission_failed(
                            "LunarLander",
                            trade_type,
                            start_time.elapsed(),
                            format!("status {}", status),
                        );
                    }
                    return Err(anyhow::anyhow!("LunarLander sendTransaction failed: {}", status));
                }
            }
            LunarLanderBackend::Quic(quic) => {
                quic.send_transaction(&body_bytes).await?;
                if crate::common::sdk_log::sdk_log_enabled() {
                    crate::common::sdk_log::log_swqos_submitted(
                        "LunarLander",
                        trade_type,
                        start_time.elapsed(),
                    );
                }
            }
        }

        let start_time = Instant::now();
        match poll_transaction_confirmation(&self.rpc_client, signature, wait_confirmation).await {
            Ok(_) => (),
            Err(e) => {
                if crate::common::sdk_log::sdk_log_enabled() {
                    println!(" signature: {:?}", signature);
                    println!(
                        " [{:width$}] {} confirmation failed: {:?}",
                        "LunarLander",
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
                "LunarLander",
                trade_type,
                start_time.elapsed(),
                width = crate::common::sdk_log::SWQOS_LABEL_WIDTH
            );
        }
        Ok(())
    }
}

impl Drop for LunarLanderClient {
    fn drop(&mut self) {
        match &self.backend {
            LunarLanderBackend::Http { stop_ping, ping_handle, .. } => {
                stop_ping.store(true, Ordering::Relaxed);
                if let Ok(mut guard) = ping_handle.try_lock() {
                    if let Some(handle) = guard.take() {
                        handle.abort();
                    }
                }
            }
            LunarLanderBackend::Quic(_) => {}
        }
    }
}

fn quic_client_options(mev_protection: bool) -> ClientOptions {
    ClientOptions { mev_protect: mev_protection, ..ClientOptions::default() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quic_options_follow_sdk_mev_protection() {
        assert!(!quic_client_options(false).mev_protect);
        assert!(quic_client_options(true).mev_protect);
    }
}
