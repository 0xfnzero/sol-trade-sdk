use crate::swqos::common::poll_transaction_confirmation;
use crate::swqos::common::serialize_transaction_and_encode;
use crate::swqos::serialization;
use rand::seq::IndexedRandom;
use reqwest::Client;
use std::{sync::Arc, time::Instant};

use std::time::Duration;
use solana_transaction_status::UiTransactionEncoding;

use anyhow::Result;
use solana_sdk::transaction::VersionedTransaction;
use crate::swqos::{SwqosType, TradeType};
use crate::swqos::SwqosClientTrait;

use crate::{common::SolanaRpcClient, constants::swqos::BLOX_TIP_ACCOUNTS};


#[derive(Clone)]
pub struct BloxrouteClient {
    pub endpoint: String,
    pub auth_token: String,
    pub rpc_client: Arc<SolanaRpcClient>,
    pub http_client: Client,
}

#[async_trait::async_trait]
impl SwqosClientTrait for BloxrouteClient {
    async fn send_transaction(&self, trade_type: TradeType, transaction: &VersionedTransaction, wait_confirmation: bool) -> Result<()> {
        self.send_transaction(trade_type, transaction, wait_confirmation).await
    }

    async fn send_transactions(&self, trade_type: TradeType, transactions: &Vec<VersionedTransaction>, wait_confirmation: bool) -> Result<()> {
        self.send_transactions(trade_type, transactions, wait_confirmation).await
    }

    fn get_tip_account(&self) -> Result<String> {
        let tip_account = *BLOX_TIP_ACCOUNTS.choose(&mut rand::rng()).or_else(|| BLOX_TIP_ACCOUNTS.first()).unwrap();
        Ok(tip_account.to_string())
    }

    fn get_swqos_type(&self) -> SwqosType {
        SwqosType::Bloxroute
    }
}

impl BloxrouteClient {
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
        Self { rpc_client: Arc::new(rpc_client), endpoint, auth_token, http_client }
    }

    pub async fn send_transaction(&self, trade_type: TradeType, transaction: &VersionedTransaction, wait_confirmation: bool) -> Result<()> {
        let start_time = Instant::now();
        let (content, signature) = serialize_transaction_and_encode(transaction, UiTransactionEncoding::Base64)?;

        // Single format! for body to avoid json! + to_string() double allocation
        let body = format!(
            r#"{{"transaction":{{"content":"{}"}},"frontRunningProtection":false,"useStakedRPCs":true}}"#,
            content
        );

        let endpoint = format!("{}/api/v2/submit", self.endpoint);
        let response_text = self.http_client.post(&endpoint)
            .body(body)
            .header("Content-Type", "application/json")
            .header("Authorization", self.auth_token.as_str())
            .send()
            .await?
            .text()
            .await?;

        // Parse with from_str to avoid extra wait from .json().await
        if let Ok(response_json) = serde_json::from_str::<serde_json::Value>(&response_text) {
            if response_json.get("result").is_some() {
                println!(" [bloxroute] {} submitted: {:?}", trade_type, start_time.elapsed());
            } else if let Some(_error) = response_json.get("error") {
                eprintln!(" [bloxroute] {} submission failed: {:?}", trade_type, _error);
            }
        } else {
            eprintln!(" [bloxroute] {} submission failed: {:?}", trade_type, response_text);
        }

        let start_time: Instant = Instant::now();
        match poll_transaction_confirmation(&self.rpc_client, signature, wait_confirmation).await {
            Ok(_) => (),
            Err(e) => {
                println!(" signature: {:?}", signature);
                println!(" [bloxroute] {} confirmation failed: {:?}", trade_type, start_time.elapsed());
                return Err(e);
            },
        }
        if wait_confirmation {
            println!(" signature: {:?}", signature);
            println!(" [bloxroute] {} confirmed: {:?}", trade_type, start_time.elapsed());
        }

        Ok(())
    }

    pub async fn send_transactions(&self, trade_type: TradeType, transactions: &Vec<VersionedTransaction>, _wait_confirmation: bool) -> Result<()> {
        let start_time = Instant::now();

        let contents = serialization::serialize_transactions_batch_sync(
            transactions.as_slice(),
            UiTransactionEncoding::Base64,
        )?;
        let entries: String = contents
            .iter()
            .map(|c| format!(r#"{{"transaction":{{"content":"{}"}}}}"#, c))
            .collect::<Vec<_>>()
            .join(",");
        let body = format!(r#"{{"entries":[{}]}}"#, entries);

        let endpoint = format!("{}/api/v2/submit-batch", self.endpoint);
        let response_text = self.http_client.post(&endpoint)
            .body(body)
            .header("Content-Type", "application/json")
            .header("Authorization", self.auth_token.as_str())
            .send()
            .await?
            .text()
            .await?;

        if let Ok(response_json) = serde_json::from_str::<serde_json::Value>(&response_text) {
            if response_json.get("result").is_some() {
                println!(" bloxroute {} submitted: {:?}", trade_type, start_time.elapsed());
            } else if let Some(_error) = response_json.get("error") {
                eprintln!(" bloxroute {} submission failed: {:?}", trade_type, _error);
            }
        }

        Ok(())
    }
}