use crate::swqos::common::{default_http_client_builder, poll_transaction_confirmation};
use bincode;
use rand::seq::IndexedRandom;
use reqwest::Client;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{sync::Arc, time::Duration, time::Instant};
use tokio::task::JoinHandle;

use crate::swqos::SwqosClientTrait;
use crate::swqos::{SwqosType, TradeType};
use anyhow::Result;
use solana_sdk::transaction::VersionedTransaction;

use crate::{common::SolanaRpcClient, constants::swqos::ZEROSLOT_TIP_ACCOUNTS};

#[derive(Clone)]
pub struct ZeroSlotClient {
    pub endpoint: String,
    pub auth_token: String,
    pub rpc_client: Arc<SolanaRpcClient>,
    pub http_client: Client,
    pub ping_handle: Arc<tokio::sync::Mutex<Option<JoinHandle<()>>>>,
    pub stop_ping: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl SwqosClientTrait for ZeroSlotClient {
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
        let tip_account = *ZEROSLOT_TIP_ACCOUNTS
            .choose(&mut rand::rng())
            .or_else(|| ZEROSLOT_TIP_ACCOUNTS.first())
            .unwrap();
        Ok(tip_account.to_string())
    }

    fn get_swqos_type(&self) -> SwqosType {
        SwqosType::ZeroSlot
    }
}

impl ZeroSlotClient {
    pub fn new(rpc_url: String, endpoint: String, auth_token: String) -> Self {
        let rpc_client = SolanaRpcClient::new(rpc_url);
        let http_client = default_http_client_builder().build().unwrap();

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
                    eprintln!("0slot ping request failed: {}", e);
                }
            }
            let mut interval = tokio::time::interval(Duration::from_secs(30)); // 30s keepalive under 65s server timeout
            loop {
                interval.tick().await;
                if stop_ping.load(Ordering::Relaxed) {
                    break;
                }
                if let Err(e) = Self::send_ping_request(&http_client, &endpoint, &auth_token).await
                {
                    if crate::common::sdk_log::sdk_log_enabled() {
                        eprintln!("0slot ping request failed: {}", e);
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

    /// Send ping request: POST with getHealth method (Keep Alive). Free operation, not counted toward TPS.
    async fn send_ping_request(
        http_client: &Client,
        endpoint: &str,
        auth_token: &str,
    ) -> Result<()> {
        let url = format!("{}/?api-key={}", endpoint, auth_token);
        let response = http_client
            .post(&url)
            .header("Content-Type", "application/json")
            .timeout(Duration::from_millis(1500))
            .body(r#"{"jsonrpc":"2.0","id":1,"method":"getHealth"}"#)
            .send()
            .await?;
        let _ = response.bytes().await;
        Ok(())
    }

    pub async fn send_transaction(
        &self,
        trade_type: TradeType,
        transaction: &VersionedTransaction,
        wait_confirmation: bool,
    ) -> Result<()> {
        let start_time = Instant::now();

        // Binary-Tx: Send raw binary transaction bytes directly
        // This is faster than JSON-RPC as it avoids unnecessary encoding/decoding
        let tx_bytes = bincode::serialize(transaction)?;

        // Build URL for Binary-Tx endpoint: {endpoint}/txb?api-key={auth_token}
        let mut url = String::with_capacity(self.endpoint.len() + self.auth_token.len() + 20);
        url.push_str(&self.endpoint);
        url.push_str("/txb?api-key=");
        url.push_str(&self.auth_token);

        // Send binary transaction directly
        let response = self
            .http_client
            .post(&url)
            .header("User-Agent", "") // Optional: 0slot recommends empty User-Agent
            .body(tx_bytes)
            .send()
            .await?;

        let status = response.status();
        let response_text = response.text().await?;

        // Binary-Tx returns JSON-RPC 2.0 format responses
        // 200: success with result field containing signature, or error field with code/message
        // 403: api-key error (null, doesn't exist, or expired)
        // 419: rate limit exceeded
        // 500: submission failed
        match status.as_u16() {
            200 => {
                if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(&response_text) {
                    if json_value.get("result").is_some() {
                        crate::common::sdk_log::log_swqos_submitted(
                            "0slot",
                            trade_type,
                            start_time.elapsed(),
                        );
                    } else if let Some(error) = json_value.get("error") {
                        let code = error
                            .get("code")
                            .and_then(|c| c.as_i64())
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| "unknown".to_string());
                        let message = error
                            .get("message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("unknown error");
                        crate::common::sdk_log::log_swqos_submission_failed(
                            "0slot",
                            trade_type,
                            start_time.elapsed(),
                            format!("code {}: {}", code, message),
                        );
                        return Err(anyhow::anyhow!("0slot Binary-Tx error: {}", message));
                    } else {
                        crate::common::sdk_log::log_swqos_submission_failed(
                            "0slot",
                            trade_type,
                            start_time.elapsed(),
                            format!("unexpected JSON: {}", response_text),
                        );
                        return Err(anyhow::anyhow!(
                            "0slot Binary-Tx unexpected JSON: {}",
                            response_text
                        ));
                    }
                } else {
                    crate::common::sdk_log::log_swqos_submission_failed(
                        "0slot",
                        trade_type,
                        start_time.elapsed(),
                        format!("invalid JSON: {}", response_text),
                    );
                    return Err(anyhow::anyhow!("0slot Binary-Tx invalid JSON: {}", response_text));
                }
            }
            403 => {
                crate::common::sdk_log::log_swqos_submission_failed(
                    "0slot",
                    trade_type,
                    start_time.elapsed(),
                    response_text.clone(),
                );
                return Err(anyhow::anyhow!("0slot API key error: {}", response_text));
            }
            419 => {
                crate::common::sdk_log::log_swqos_submission_failed(
                    "0slot",
                    trade_type,
                    start_time.elapsed(),
                    response_text.clone(),
                );
                return Err(anyhow::anyhow!("0slot rate limit exceeded"));
            }
            500 => {
                crate::common::sdk_log::log_swqos_submission_failed(
                    "0slot",
                    trade_type,
                    start_time.elapsed(),
                    "submission failed".to_string(),
                );
                return Err(anyhow::anyhow!("0slot transaction submission failed"));
            }
            _ => {
                crate::common::sdk_log::log_swqos_submission_failed(
                    "0slot",
                    trade_type,
                    start_time.elapsed(),
                    format!("status {} body: {}", status, response_text),
                );
                return Err(anyhow::anyhow!(
                    "0slot Binary-Tx failed with status {}: {}",
                    status,
                    response_text
                ));
            }
        }

        // Get transaction signature from the transaction for confirmation polling
        let signature = transaction.signatures[0];

        let start_time = Instant::now();
        match poll_transaction_confirmation(&self.rpc_client, signature, wait_confirmation).await {
            Ok(_) => (),
            Err(e) => {
                println!(" signature: {:?}", signature);
                println!(
                    " [{:width$}] {} confirmation failed: {:?}",
                    "0slot",
                    trade_type,
                    start_time.elapsed(),
                    width = crate::common::sdk_log::SWQOS_LABEL_WIDTH
                );
                return Err(e);
            }
        }
        if wait_confirmation {
            println!(" signature: {:?}", signature);
            println!(
                " [{:width$}] {} confirmed: {:?}",
                "0slot",
                trade_type,
                start_time.elapsed(),
                width = crate::common::sdk_log::SWQOS_LABEL_WIDTH
            );
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

impl Drop for ZeroSlotClient {
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
