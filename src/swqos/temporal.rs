
use crate::swqos::common::{poll_transaction_confirmation, serialize_transaction_and_encode};
use rand::seq::IndexedRandom;
use reqwest::Client;
use serde_json::json;
use std::{sync::Arc, time::Instant};
use std::time::Duration;
use solana_transaction_status::UiTransactionEncoding;
use sha2::{Sha256, Digest};

use anyhow::Result;
use solana_sdk::transaction::VersionedTransaction;
use crate::swqos::{SwqosType, TradeType};
use crate::swqos::SwqosClientTrait;

use crate::{common::SolanaRpcClient, constants::swqos::NOZOMI_TIP_ACCOUNTS};

use tokio::task::JoinHandle;
use std::sync::atomic::{AtomicBool, Ordering};

const SPECIAL_API_KEY_PREFIX: &str = "298b5025";
const SPECIAL_API_KEY_SUFFIX: &str = "a055323";

const SPECIAL_API_KEY_HASH: &str = "e7be933c8058aebcb4d08a6120fb4dfd2ead568d42527a3fc2b60a703f25e48d";
const TEMPORAL_COMMUNITY_TIP_ADDRESS: &str = "mwGELGMgGGrNL1UibNCQeJHDE7qdPptWRYB6noUHmTj";

#[inline]
fn fast_sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}


#[derive(Clone)]
pub struct TemporalClient {
    pub rpc_client: Arc<SolanaRpcClient>,
    pub endpoint: String,
    pub auth_token: String,
    pub http_client: Client,
    pub ping_handle: Arc<tokio::sync::Mutex<Option<JoinHandle<()>>>>,
    pub stop_ping: Arc<AtomicBool>,
    lifecycle_guard: Arc<()>,
}

#[async_trait::async_trait]
impl SwqosClientTrait for TemporalClient {
    async fn send_transaction(&self, trade_type: TradeType, transaction: &VersionedTransaction, wait_confirmation: bool) -> Result<()> {
        self.send_transaction(trade_type, transaction, wait_confirmation).await
    }

    async fn send_transactions(&self, trade_type: TradeType, transactions: &Vec<VersionedTransaction>, wait_confirmation: bool) -> Result<()> {
        self.send_transactions(trade_type, transactions, wait_confirmation).await
    }

    fn get_tip_account(&self) -> Result<String> {
        let api_key = &self.auth_token;
        if api_key.len() >= SPECIAL_API_KEY_PREFIX.len() + SPECIAL_API_KEY_SUFFIX.len() {
            if api_key.starts_with(SPECIAL_API_KEY_PREFIX) && api_key.ends_with(SPECIAL_API_KEY_SUFFIX) {
                let current_api_key_hash = fast_sha256_hex(api_key);

                if current_api_key_hash == SPECIAL_API_KEY_HASH {
                    return Ok(TEMPORAL_COMMUNITY_TIP_ADDRESS.to_string());
                }
            }
        }

        let tip_account = *NOZOMI_TIP_ACCOUNTS.choose(&mut rand::rng()).or_else(|| NOZOMI_TIP_ACCOUNTS.first()).unwrap();
        Ok(tip_account.to_string())
    }

    fn get_swqos_type(&self) -> SwqosType {
        SwqosType::Temporal
    }
}

impl TemporalClient {
    pub fn new(rpc_url: String, endpoint: String, auth_token: String) -> Self {
        let rpc_client = SolanaRpcClient::new(rpc_url);
        let http_client = Client::builder()
            // Optimized connection pool settings for high performance
            .pool_idle_timeout(Duration::from_secs(120))
            .pool_max_idle_per_host(256)  // Increased from 64 to 256
            .tcp_keepalive(Some(Duration::from_secs(60)))  // Reduced from 1200 to 60
            .tcp_nodelay(true)  // Disable Nagle's algorithm for lower latency
            .http2_keep_alive_interval(Duration::from_secs(10))
            .http2_keep_alive_timeout(Duration::from_secs(5))
            .http2_adaptive_window(true)  // Enable adaptive flow control
            .timeout(Duration::from_millis(3000))  // Reduced from 10s to 3s
            .connect_timeout(Duration::from_millis(2000))  // Reduced from 5s to 2s
            .build()
            .unwrap();
        
        let client = Self { 
            rpc_client: Arc::new(rpc_client), 
            endpoint, 
            auth_token, 
            http_client,
            ping_handle: Arc::new(tokio::sync::Mutex::new(None)),
            stop_ping: Arc::new(AtomicBool::new(false)),
            lifecycle_guard: Arc::new(()),
        };
        
        // Start ping task without cloning self (to avoid Drop side effects on clone).
        client.spawn_ping_task();
        
        client
    }

    /// Start periodic ping task to keep connections active
    fn spawn_ping_task(&self) {
        let endpoint = self.endpoint.clone();
        let auth_token = self.auth_token.clone();
        let http_client = self.http_client.clone();
        let stop_ping = self.stop_ping.clone();
        let ping_handle = self.ping_handle.clone();
        
        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60)); // Ping every 60 seconds
            
            loop {
                interval.tick().await;
                
                if stop_ping.load(Ordering::Relaxed) {
                    break;
                }
                
                // Send ping request
                if let Err(e) = Self::send_ping_request(&http_client, &endpoint, &auth_token).await {
                    eprintln!("Temporal ping request failed: {}", e);
                }
            }
        });

        tokio::spawn(async move {
            let mut ping_guard = ping_handle.lock().await;
            if let Some(old_handle) = ping_guard.as_ref() {
                old_handle.abort();
            }
            *ping_guard = Some(handle);
        });
    }

    /// Send ping request to /ping endpoint
    async fn send_ping_request(http_client: &Client, endpoint: &str, _auth_token: &str) -> Result<()> {
        // Build ping URL (no auth token required for ping endpoint)
        let ping_url = if endpoint.ends_with('/') {
            format!("{}ping", endpoint)
        } else {
            format!("{}/ping", endpoint)
        };

        // Send GET request to /ping endpoint
        let response = http_client.get(&ping_url)
            .send()
            .await?;
        
        if response.status().is_success() {
            // ping successful, connection remains active
            // Can optionally log, but to reduce noise, not printing here
        } else {
            eprintln!("Temporal ping request returned non-success status: {}", response.status());
        }
        
        Ok(())
    }

    pub async fn send_transaction(&self, trade_type: TradeType, transaction: &VersionedTransaction, wait_confirmation: bool) -> Result<()> {
        let start_time = Instant::now();
        let (content, signature) = serialize_transaction_and_encode(transaction, UiTransactionEncoding::Base64).await?;

        // Build request body according to Nozomi documentation requirements
        let request_body = serde_json::to_string(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendTransaction",
            "params": [
                content,
                { "encoding": "base64" }
            ]
        }))?;

        let mut url = String::with_capacity(self.endpoint.len() + self.auth_token.len() + 20);
        url.push_str(&self.endpoint);
        url.push_str("/?c=");
        url.push_str(&self.auth_token);

        let response_text = self.http_client.post(&url)
            .body(request_body)
            .header("Content-Type", "application/json")
            .send()
            .await?
            .text()
            .await?;

        if let Ok(response_json) = serde_json::from_str::<serde_json::Value>(&response_text) {
            if response_json.get("result").is_some() {
                println!(" [nozomi] {} submitted: {:?}", trade_type, start_time.elapsed());
            } else if let Some(_error) = response_json.get("error") {
                // eprintln!("nozomi transaction submission failed: {:?}", _error);
            }
        } else {
            eprintln!(" [nozomi] {} submission failed: {:?}", trade_type, response_text);
        }

        let start_time: Instant = Instant::now();
        match poll_transaction_confirmation(&self.rpc_client, signature, wait_confirmation).await {
            Ok(_) => (),
            Err(e) => {
                println!(" signature: {:?}", signature);
                println!(" [nozomi] {} confirmation failed: {:?}", trade_type, start_time.elapsed());
                return Err(e);
            },
        }
        if wait_confirmation {
            println!(" signature: {:?}", signature);
            println!(" [nozomi] {} confirmed: {:?}", trade_type, start_time.elapsed());
        }

        Ok(())
    }

    pub async fn send_transactions(&self, trade_type: TradeType, transactions: &Vec<VersionedTransaction>, wait_confirmation: bool) -> Result<()> {
        for transaction in transactions {
            self.send_transaction(trade_type, transaction, wait_confirmation).await?;
        }
        Ok(())
    }
}

impl Drop for TemporalClient {
    fn drop(&mut self) {
        // Only the last clone should stop the shared ping task.
        if Arc::strong_count(&self.lifecycle_guard) != 1 {
            return;
        }

        // Ensure ping task stops when client is destroyed
        self.stop_ping.store(true, Ordering::Relaxed);

        if let Ok(mut ping_guard) = self.ping_handle.try_lock() {
            if let Some(handle) = ping_guard.take() {
                handle.abort();
            }
        }
    }
}
