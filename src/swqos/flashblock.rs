use crate::swqos::common::{
    default_http_client_builder, poll_transaction_confirmation, serialize_transaction_and_encode,
};
use rand::seq::IndexedRandom;
use reqwest::Client;
use serde_json::json;
use std::{sync::Arc, time::Instant};

use solana_transaction_status::UiTransactionEncoding;

use crate::swqos::SwqosClientTrait;
use crate::swqos::{SwqosType, TradeType};
use anyhow::Result;
use solana_sdk::transaction::VersionedTransaction;

use crate::{common::SolanaRpcClient, constants::swqos::FLASHBLOCK_TIP_ACCOUNTS};

#[derive(Clone)]
pub struct FlashBlockClient {
    pub endpoint: String,
    pub auth_token: String,
    pub rpc_client: Arc<SolanaRpcClient>,
    pub http_client: Client,
}

#[async_trait::async_trait]
impl SwqosClientTrait for FlashBlockClient {
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
        let tip_account = *FLASHBLOCK_TIP_ACCOUNTS
            .choose(&mut rand::rng())
            .or_else(|| FLASHBLOCK_TIP_ACCOUNTS.first())
            .unwrap();
        Ok(tip_account.to_string())
    }

    fn get_swqos_type(&self) -> SwqosType {
        SwqosType::FlashBlock
    }
}

impl FlashBlockClient {
    pub fn new(rpc_url: String, endpoint: String, auth_token: String) -> Self {
        let rpc_client = SolanaRpcClient::new(rpc_url);
        let http_client = default_http_client_builder().build().unwrap();
        Self { rpc_client: Arc::new(rpc_client), endpoint, auth_token, http_client }
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

        // FlashBlock API format
        let request_body = serde_json::to_string(&json!({
            "transactions": [content]
        }))?;

        let url = format!("{}/api/v2/submit-batch", self.endpoint);

        // Send request to FlashBlock
        let response_text = self
            .http_client
            .post(&url)
            .body(request_body)
            .header("Authorization", &self.auth_token)
            .header("Content-Type", "application/json")
            .header("Connection", "keep-alive")
            .header("Keep-Alive", "timeout=30, max=1000")
            .send()
            .await?
            .text()
            .await?;

        // Parse response
        if let Ok(response_json) = serde_json::from_str::<serde_json::Value>(&response_text) {
            if response_json.get("success").is_some() || response_json.get("result").is_some() {
                println!(" [FlashBlock] {} submitted: {:?}", trade_type, start_time.elapsed());
            } else if let Some(_error) = response_json.get("error") {
                eprintln!(" [FlashBlock] {} submission failed: {:?}", trade_type, _error);
            }
        } else {
            eprintln!(" [FlashBlock] {} submission failed: {:?}", trade_type, response_text);
        }

        let start_time: Instant = Instant::now();
        match poll_transaction_confirmation(&self.rpc_client, signature, wait_confirmation).await {
            Ok(_) => (),
            Err(e) => {
                println!(" signature: {:?}", signature);
                println!(
                    " [FlashBlock] {} confirmation failed: {:?}",
                    trade_type,
                    start_time.elapsed()
                );
                return Err(e);
            }
        }
        if wait_confirmation {
            println!(" signature: {:?}", signature);
            println!(" [FlashBlock] {} confirmed: {:?}", trade_type, start_time.elapsed());
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
