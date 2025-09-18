use crate::swqos::common::{poll_transaction_confirmation, serialize_transaction_and_encode, FormatBase64VersionedTransaction};
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
    async fn send_transaction(&self, trade_type: TradeType, transaction: &VersionedTransaction) -> Result<()> {
        self.send_transaction(trade_type, transaction).await
    }

    async fn send_transactions(&self, trade_type: TradeType, transactions: &Vec<VersionedTransaction>) -> Result<()> {
        self.send_transactions(trade_type, transactions).await
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
            .pool_idle_timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(64)
            .tcp_keepalive(Some(Duration::from_secs(1200)))
            .http2_keep_alive_interval(Duration::from_secs(15))
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        Self { rpc_client: Arc::new(rpc_client), endpoint, auth_token, http_client }
    }

    pub async fn send_transaction(&self, trade_type: TradeType, transaction: &VersionedTransaction) -> Result<()> {
        let start_time = Instant::now();
        let (content, signature) = serialize_transaction_and_encode(transaction, UiTransactionEncoding::Base64).await?;

        let body = serde_json::json!({
            "transaction": {
                "content": content,
            },
            "frontRunningProtection": false,
            "useStakedRPCs": true,
        });

        let endpoint = format!("{}/api/v2/submit", self.endpoint);
        let response_text = self.http_client.post(&endpoint)
            .body(body.to_string())
            .header("Content-Type", "application/json")
            .header("Authorization", self.auth_token.clone())
            .send()
            .await?
            .text()
            .await?;

        // 5. Use `serde_json::from_str()` to parse JSON, reducing extra wait from `.json().await?`
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
        match poll_transaction_confirmation(&self.rpc_client, signature).await {
            Ok(_) => (),
            Err(e) => {
                println!(" signature: {:?}", signature);
                println!(" [bloxroute] {} confirmation failed: {:?}", trade_type, start_time.elapsed());
                return Err(e);
            },
        }
        println!(" signature: {:?}", signature);
        println!(" [bloxroute] {} confirmed: {:?}", trade_type, start_time.elapsed());

        Ok(())
    }

    pub async fn send_transactions(&self, trade_type: TradeType, transactions: &Vec<VersionedTransaction>) -> Result<()> {
        let start_time = Instant::now();

        let body = serde_json::json!({
            "entries":  transactions
                .iter()
                .map(|tx| {
                    serde_json::json!({
                        "transaction": {
                            "content": tx.to_base64_string(),
                        },
                    })
                })
                .collect::<Vec<_>>(),
        });

        let endpoint = format!("{}/api/v2/submit-batch", self.endpoint);
        let response_text = self.http_client.post(&endpoint)
            .body(body.to_string())
            .header("Content-Type", "application/json")
            .header("Authorization", self.auth_token.clone())
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