//! HelloMoon Lunar Lander SWQOS client.
//!
//! High-performance transaction landing service with keep-alive ping.
//! Auth via `?api-key=` query parameter. Minimum tip: 0.001 SOL.

use crate::swqos::common::{default_http_client_builder, poll_transaction_confirmation, serialize_transaction_and_encode};
use rand::seq::IndexedRandom;
use reqwest::Client;
use serde_json::json;
use std::{sync::Arc, time::Instant};
use std::sync::atomic::{AtomicBool, Ordering};

use std::time::Duration;
use solana_transaction_status::UiTransactionEncoding;

use anyhow::Result;
use solana_sdk::transaction::VersionedTransaction;
use crate::swqos::{SwqosType, TradeType};
use crate::swqos::SwqosClientTrait;

use crate::{common::SolanaRpcClient, constants::swqos::LUNARLANDER_TIP_ACCOUNTS};


#[derive(Clone)]
pub struct LunarLanderClient {
    pub endpoint: String,
    pub api_key: String,
    pub rpc_client: Arc<SolanaRpcClient>,
    pub http_client: Client,
    stop_ping: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl SwqosClientTrait for LunarLanderClient {
    async fn send_transaction(&self, trade_type: TradeType, transaction: &VersionedTransaction, wait_confirmation: bool) -> Result<()> {
        self.send_transaction(trade_type, transaction, wait_confirmation).await
    }

    async fn send_transactions(&self, trade_type: TradeType, transactions: &Vec<VersionedTransaction>, wait_confirmation: bool) -> Result<()> {
        self.send_transactions(trade_type, transactions, wait_confirmation).await
    }

    fn get_tip_account(&self) -> Result<String> {
        let tip_account = *LUNARLANDER_TIP_ACCOUNTS.choose(&mut rand::rng()).or_else(|| LUNARLANDER_TIP_ACCOUNTS.first()).unwrap();
        Ok(tip_account.to_string())
    }

    fn get_swqos_type(&self) -> SwqosType {
        SwqosType::LunarLander
    }
}

impl LunarLanderClient {
    /// Derive the ping URL from the send endpoint by replacing the last path segment.
    fn ping_url(endpoint: &str, api_key: &str) -> String {
        // Find the last '/' that is part of the path (after "://")
        let scheme_end = endpoint.find("://").map(|i| i + 3).unwrap_or(0);
        let base = endpoint[scheme_end..].rfind('/')
            .map(|i| &endpoint[..scheme_end + i])
            .unwrap_or(endpoint);
        format!("{}/ping?api-key={}", base, api_key)
    }

    /// Derive the send URL with the API key query parameter.
    fn send_url(endpoint: &str, api_key: &str) -> String {
        let separator = if endpoint.contains('?') { '&' } else { '?' };
        format!("{}{}api-key={}", endpoint, separator, api_key)
    }

    pub fn new(rpc_url: String, endpoint: String, api_key: String) -> Self {
        let rpc_client = SolanaRpcClient::new(rpc_url);
        let http_client = default_http_client_builder().build().unwrap();

        let stop_ping = Arc::new(AtomicBool::new(false));

        let client = Self {
            rpc_client: Arc::new(rpc_client),
            endpoint: endpoint.clone(),
            api_key: api_key.clone(),
            http_client: http_client.clone(),
            stop_ping: stop_ping.clone(),
        };

        // Start ping task
        let client_clone = client.clone();
        tokio::spawn(async move {
            client_clone.start_ping_task().await;
        });

        client
    }

    /// Start periodic ping task to keep connections active.
    /// GET /ping every 30s on the same TCP connection (recommended 30-45s by HelloMoon).
    async fn start_ping_task(&self) {
        let ping_url = Self::ping_url(&self.endpoint, &self.api_key);
        let http_client = self.http_client.clone();
        let stop_ping = self.stop_ping.clone();

        tokio::spawn(async move {
            // Immediate first ping to warm connection and reduce first-submit cold start latency
            if let Ok(resp) = http_client.get(&ping_url).timeout(Duration::from_millis(1500)).send().await {
                let status = resp.status();
                let _ = resp.bytes().await;
                if !status.is_success() && crate::common::sdk_log::sdk_log_enabled() {
                    eprintln!(" [lunarlander] ping failed with status: {}", status);
                }
            }
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                if stop_ping.load(Ordering::Relaxed) {
                    break;
                }
                match http_client.get(&ping_url).timeout(Duration::from_millis(1500)).send().await {
                    Ok(response) => {
                        let status = response.status();
                        let _ = response.bytes().await;
                        if !status.is_success() && crate::common::sdk_log::sdk_log_enabled() {
                            eprintln!(" [lunarlander] ping failed with status: {}", status);
                        }
                    }
                    Err(e) => {
                        if crate::common::sdk_log::sdk_log_enabled() {
                            eprintln!(" [lunarlander] ping request error: {:?}", e);
                        }
                    }
                }
            }
        });
    }

    pub async fn send_transaction(&self, trade_type: TradeType, transaction: &VersionedTransaction, wait_confirmation: bool) -> Result<()> {
        let start_time = Instant::now();
        let (content, signature) = serialize_transaction_and_encode(transaction, UiTransactionEncoding::Base64)?;

        let request_body = serde_json::to_string(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendTransaction",
            "params": [
                content,
                { "encoding": "base64" }
            ]
        }))?;

        let url = Self::send_url(&self.endpoint, &self.api_key);

        let response_text = self.http_client.post(&url)
            .body(request_body)
            .header("Content-Type", "application/json")
            .header("Connection", "keep-alive")
            .header("Keep-Alive", "timeout=30, max=1000")
            .send()
            .await?
            .text()
            .await?;

        // Parse response
        if let Ok(response_json) = serde_json::from_str::<serde_json::Value>(&response_text) {
            if crate::common::sdk_log::sdk_log_enabled() {
                if response_json.get("result").is_some() {
                    println!(" [lunarlander] {} submitted: {:?}", trade_type, start_time.elapsed());
                } else if let Some(error) = response_json.get("error") {
                    eprintln!(" [lunarlander] {} submission failed: {:?}", trade_type, error);
                }
            }
        } else if crate::common::sdk_log::sdk_log_enabled() {
            eprintln!(" [lunarlander] {} submission failed: {:?}", trade_type, response_text);
        }

        let start_time: Instant = Instant::now();
        match poll_transaction_confirmation(&self.rpc_client, signature, wait_confirmation).await {
            Ok(_) => (),
            Err(e) => {
                if crate::common::sdk_log::sdk_log_enabled() {
                    println!(" signature: {:?}", signature);
                    println!(" [lunarlander] {} confirmation failed: {:?}", trade_type, start_time.elapsed());
                }
                return Err(e);
            },
        }
        if wait_confirmation && crate::common::sdk_log::sdk_log_enabled() {
            println!(" signature: {:?}", signature);
            println!(" [lunarlander] {} confirmed: {:?}", trade_type, start_time.elapsed());
        }

        Ok(())
    }

    pub async fn send_transactions(&self, trade_type: TradeType, transactions: &Vec<VersionedTransaction>, wait_confirmation: bool) -> Result<()> {
        for transaction in transactions {
            self.send_transaction(trade_type, transaction, wait_confirmation).await?;
        }
        Ok(())
    }

    /// Stop the ping task
    pub fn stop_ping_task(&self) {
        self.stop_ping.store(true, Ordering::Relaxed);
    }
}

impl Drop for LunarLanderClient {
    fn drop(&mut self) {
        // Stop ping task when client is dropped
        self.stop_ping.store(true, Ordering::Relaxed);
    }
}
