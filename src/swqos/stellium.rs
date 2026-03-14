use crate::swqos::common::{
    default_http_client_builder, poll_transaction_confirmation, serialize_transaction_and_encode,
};
use rand::seq::IndexedRandom;
use reqwest::Client;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{sync::Arc, time::Instant};

use solana_transaction_status::UiTransactionEncoding;
use std::time::Duration;

use crate::swqos::SwqosClientTrait;
use crate::swqos::{SwqosType, TradeType};
use anyhow::Result;
use solana_sdk::transaction::VersionedTransaction;

use crate::{common::SolanaRpcClient, constants::swqos::STELLIUM_TIP_ACCOUNTS};

#[derive(Clone)]
pub struct StelliumClient {
    pub endpoint: String,
    pub auth_token: String,
    pub rpc_client: Arc<SolanaRpcClient>,
    pub http_client: Client,
    keep_alive_running: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl SwqosClientTrait for StelliumClient {
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
        let tip_account = *STELLIUM_TIP_ACCOUNTS
            .choose(&mut rand::rng())
            .or_else(|| STELLIUM_TIP_ACCOUNTS.first())
            .unwrap();
        Ok(tip_account.to_string())
    }

    fn get_swqos_type(&self) -> SwqosType {
        SwqosType::Stellium
    }
}

impl StelliumClient {
    pub fn new(rpc_url: String, endpoint: String, auth_token: String) -> Self {
        let rpc_client = SolanaRpcClient::new(rpc_url);
        let http_client = default_http_client_builder().build().unwrap();

        let keep_alive_running = Arc::new(AtomicBool::new(true));

        let client = Self {
            rpc_client: Arc::new(rpc_client),
            endpoint: endpoint.clone(),
            auth_token: auth_token.clone(),
            http_client: http_client.clone(),
            keep_alive_running: keep_alive_running.clone(),
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
        let stop_ping = self.keep_alive_running.clone();

        tokio::spawn(async move {
            // Immediate first ping to warm connection and reduce first-submit cold start latency
            let url = format!("{}/{}", endpoint, auth_token);
            if let Ok(resp) =
                http_client.get(&url).timeout(Duration::from_millis(1500)).send().await
            {
                let status = resp.status();
                let _ = resp.bytes().await;
                if !status.is_success() && crate::common::sdk_log::sdk_log_enabled() {
                    eprintln!(" [Stellium] Ping failed with status: {}", status);
                }
            }
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                if stop_ping.load(Ordering::Relaxed) {
                    break;
                }
                let url = format!("{}/{}", endpoint, auth_token);
                match http_client.get(&url).timeout(Duration::from_millis(1500)).send().await {
                    Ok(response) => {
                        let status = response.status();
                        let _ = response.bytes().await;
                        if !status.is_success() && crate::common::sdk_log::sdk_log_enabled() {
                            eprintln!(" [Stellium] Ping failed with status: {}", status);
                        }
                    }
                    Err(e) => {
                        if crate::common::sdk_log::sdk_log_enabled() {
                            eprintln!(" [Stellium] Ping request error: {:?}", e);
                        }
                    }
                }
            }
        });
    }

    pub async fn send_transaction(
        &self,
        trade_type: TradeType,
        transaction: &VersionedTransaction,
        wait_confirmation: bool,
    ) -> Result<()> {
        let start_time = Instant::now();
        let (content, signature) =
            serialize_transaction_and_encode(transaction, UiTransactionEncoding::Base64)?;

        // Stellium uses standard Solana sendTransaction format
        let request_body = serde_json::to_string(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendTransaction",
            "params": [
                content,
                { "encoding": "base64" }
            ]
        }))?;

        // Build the URL with the API key
        let url = format!("{}/{}", self.endpoint, self.auth_token);

        // Send request to Stellium
        let response_text = self
            .http_client
            .post(&url)
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
                    println!(" [Stellium] {} submitted: {:?}", trade_type, start_time.elapsed());
                } else if let Some(_error) = response_json.get("error") {
                    eprintln!(" [Stellium] {} submission failed: {:?}", trade_type, _error);
                }
            }
        } else if crate::common::sdk_log::sdk_log_enabled() {
            eprintln!(" [Stellium] {} submission failed: {:?}", trade_type, response_text);
        }

        let start_time: Instant = Instant::now();
        match poll_transaction_confirmation(&self.rpc_client, signature, wait_confirmation).await {
            Ok(_) => (),
            Err(e) => {
                if crate::common::sdk_log::sdk_log_enabled() {
                    println!(" signature: {:?}", signature);
                    println!(
                        " [Stellium] {} confirmation failed: {:?}",
                        trade_type,
                        start_time.elapsed()
                    );
                }
                return Err(e);
            }
        }
        if wait_confirmation && crate::common::sdk_log::sdk_log_enabled() {
            println!(" signature: {:?}", signature);
            println!(" [Stellium] {} confirmed: {:?}", trade_type, start_time.elapsed());
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

    /// Stop the ping task
    pub fn stop_ping_task(&self) {
        self.keep_alive_running.store(false, Ordering::Relaxed);
    }
}

impl Drop for StelliumClient {
    fn drop(&mut self) {
        // Stop ping task when client is dropped
        self.keep_alive_running.store(false, Ordering::Relaxed);
    }
}
