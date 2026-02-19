use crate::swqos::common::{poll_transaction_confirmation, serialize_transaction_and_encode};
use rand::seq::IndexedRandom;
use reqwest::Client;
use serde_json::json;
use std::{sync::Arc, time::Instant};

use std::time::Duration;
use solana_transaction_status::UiTransactionEncoding;

use anyhow::Result;
use solana_sdk::transaction::VersionedTransaction;
use crate::swqos::{SwqosType, TradeType};
use crate::swqos::SwqosClientTrait;

use crate::{common::SolanaRpcClient, constants::swqos::NODE1_TIP_ACCOUNTS};

use tokio::task::JoinHandle;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone)]
pub struct Node1Client {
    pub endpoint: String,
    pub auth_token: String,
    pub rpc_client: Arc<SolanaRpcClient>,
    pub http_client: Client,
    pub ping_handle: Arc<tokio::sync::Mutex<Option<JoinHandle<()>>>>,
    pub stop_ping: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl SwqosClientTrait for Node1Client {
    async fn send_transaction(&self, trade_type: TradeType, transaction: &VersionedTransaction, wait_confirmation: bool) -> Result<()> {
        self.send_transaction(trade_type, transaction, wait_confirmation).await
    }

    async fn send_transactions(&self, trade_type: TradeType, transactions: &Vec<VersionedTransaction>, wait_confirmation: bool) -> Result<()> {
        self.send_transactions(trade_type, transactions, wait_confirmation).await
    }

    fn get_tip_account(&self) -> Result<String> {
        let tip_account = *NODE1_TIP_ACCOUNTS.choose(&mut rand::rng()).or_else(|| NODE1_TIP_ACCOUNTS.first()).unwrap();
        Ok(tip_account.to_string())
    }

    fn get_swqos_type(&self) -> SwqosType {
        SwqosType::Node1
    }
}

impl Node1Client {
    pub fn new(rpc_url: String, endpoint: String, auth_token: String) -> Self {
        let rpc_client = SolanaRpcClient::new(rpc_url);
        let http_client = Client::builder()
            // Optimized connection pool settings for high performance
            .pool_idle_timeout(Duration::from_secs(300))
            .pool_max_idle_per_host(4)
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
                eprintln!("Node1 ping request failed: {}", e);
            }
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                if stop_ping.load(Ordering::Relaxed) {
                    break;
                }
                if let Err(e) = Self::send_ping_request(&http_client, &endpoint, &auth_token).await {
                    eprintln!("Node1 ping request failed: {}", e);
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

    /// Send ping request to /ping endpoint
    async fn send_ping_request(http_client: &Client, endpoint: &str, _auth_token: &str) -> Result<()> {
        // Build ping URL
        let ping_url = if endpoint.ends_with('/') {
            format!("{}ping", endpoint)
        } else {
            format!("{}/ping", endpoint)
        };

        // Short timeout for ping; consume body so connection is returned to pool for reuse by submit
        let response = http_client.get(&ping_url)
            .timeout(Duration::from_millis(1500))
            .send()
            .await?;
        let status = response.status();
        let _ = response.bytes().await;
        if !status.is_success() {
            eprintln!("Node1 ping request returned non-success status: {}", status);
        }
        Ok(())
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
                { "encoding": "base64", "skipPreflight": true }
            ]
        }))?;

        // Node1 uses api-key header instead of URL parameter
        let response_text = self.http_client.post(&self.endpoint)
            .body(request_body)
            .header("Content-Type", "application/json")
            .header("api-key", &self.auth_token)
            .send()
            .await?
            .text()
            .await?;

        // Parse JSON response
        if let Ok(response_json) = serde_json::from_str::<serde_json::Value>(&response_text) {
            if response_json.get("result").is_some() {
                println!(" [node1] {} submitted: {:?}", trade_type, start_time.elapsed());
            } else if let Some(_error) = response_json.get("error") {
                eprintln!(" [node1] {} submission failed: {:?}", trade_type, _error);
            }
        } else {
            eprintln!(" [node1] {} submission failed: {:?}", trade_type, response_text);
        }

        let start_time: Instant = Instant::now();
        match poll_transaction_confirmation(&self.rpc_client, signature, wait_confirmation).await {
            Ok(_) => (),
            Err(e) => {
                println!(" signature: {:?}", signature);
                println!(" [node1] {} confirmation failed: {:?}", trade_type, start_time.elapsed());
                return Err(e);
            },
        }
        if wait_confirmation {
            println!(" signature: {:?}", signature);
            println!(" [node1] {} confirmed: {:?}", trade_type, start_time.elapsed());
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

impl Drop for Node1Client {
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
