//! Fastlane SWQoS client: v2 API only (POST /v2/sendTransaction, body = binary bincode).
//! Endpoints: ny http://64.130.37.195:8080, fra http://70.40.184.37:8080.

use crate::swqos::common::{default_http_client_builder, poll_transaction_confirmation};
use anyhow::Result;
use bincode::serialize as bincode_serialize;
use rand::seq::IndexedRandom;
use reqwest::Client;
use solana_client::rpc_client::SerializableTransaction;
use solana_sdk::transaction::VersionedTransaction;
use std::sync::Arc;
use std::time::Instant;

use crate::swqos::{SwqosClientTrait, SwqosType, TradeType};
use crate::{common::SolanaRpcClient, constants::swqos::FASTLANE_TIP_ACCOUNTS};

/// Fastlane v2 submit path (binary bincode, no Base64).
const FASTLANE_V2_PATH: &str = "/v2/sendTransaction";

#[derive(Clone)]
pub struct FastlaneClient {
    /// Base URL including port, e.g. http://64.130.37.195:8080
    pub base_url: String,
    /// Optional API key for request header api-key (empty = no auth).
    pub api_key: String,
    pub rpc_client: Arc<SolanaRpcClient>,
    pub http_client: Client,
}

#[async_trait::async_trait]
impl SwqosClientTrait for FastlaneClient {
    async fn send_transaction(&self, trade_type: TradeType, transaction: &VersionedTransaction, wait_confirmation: bool) -> Result<()> {
        self.send_transaction_impl(trade_type, transaction, wait_confirmation).await
    }

    async fn send_transactions(&self, trade_type: TradeType, transactions: &Vec<VersionedTransaction>, wait_confirmation: bool) -> Result<()> {
        for tx in transactions {
            self.send_transaction_impl(trade_type, tx, wait_confirmation).await?;
        }
        Ok(())
    }

    fn get_tip_account(&self) -> Result<String> {
        let tip_account = *FASTLANE_TIP_ACCOUNTS
            .choose(&mut rand::rng())
            .or_else(|| FASTLANE_TIP_ACCOUNTS.first())
            .unwrap();
        Ok(tip_account.to_string())
    }

    fn get_swqos_type(&self) -> SwqosType {
        SwqosType::Fastlane
    }
}

impl FastlaneClient {
    pub fn new(rpc_url: String, base_url: String, api_key: String) -> Self {
        let rpc_client = SolanaRpcClient::new(rpc_url);
        let http_client = default_http_client_builder().build().unwrap();
        Self {
            base_url,
            api_key,
            rpc_client: Arc::new(rpc_client),
            http_client,
        }
    }

    fn v2_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        format!("{}{}", base, FASTLANE_V2_PATH)
    }

    /// Send transaction via Fastlane v2 API: POST /v2/sendTransaction, body = bincode.
    pub async fn send_transaction_impl(
        &self,
        trade_type: TradeType,
        transaction: &VersionedTransaction,
        wait_confirmation: bool,
    ) -> Result<()> {
        let start_time = Instant::now();
        let signature = transaction.get_signature();

        let body_bytes = bincode_serialize(transaction)
            .map_err(|e| anyhow::anyhow!("Fastlane bincode serialize failed: {}", e))?;

        let url = self.v2_url();
        let mut req = self.http_client.post(&url).header("Content-Type", "application/octet-stream").body(body_bytes);
        if !self.api_key.is_empty() {
            req = req.header("api-key", self.api_key.as_str());
        }

        let response = req.send().await?;
        let status = response.status();
        let _ = response.bytes().await;
        if status.is_success() {
            println!(" [fastlane] {} submitted: {:?}", trade_type, start_time.elapsed());
        } else {
            eprintln!(" [fastlane] {} submission failed: status {}", trade_type, status);
            return Err(anyhow::anyhow!("Fastlane sendTransaction failed: {}", status));
        }

        let start_confirm = Instant::now();
        match poll_transaction_confirmation(&self.rpc_client, *signature, wait_confirmation).await {
            Ok(_) => (),
            Err(e) => {
                println!(" signature: {:?}", signature);
                println!(" [fastlane] {} confirmation failed: {:?}", trade_type, start_confirm.elapsed());
                return Err(e);
            }
        }
        if wait_confirmation {
            println!(" signature: {:?}", signature);
            println!(" [fastlane] {} confirmed: {:?}", trade_type, start_confirm.elapsed());
        }

        Ok(())
    }
}
