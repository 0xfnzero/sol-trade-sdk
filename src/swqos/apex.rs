use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use bytes::Bytes;
use orbitflare_apex::{ApexSenderClient, ClientOptions};
use rand::seq::IndexedRandom;
use reqwest::Client;
use serde_json::Value;
use solana_sdk::transaction::VersionedTransaction;
use tokio::task::JoinHandle;
use wincode::serialize as wincode_serialize;

use crate::common::SolanaRpcClient;
use crate::constants::swqos::APEX_TIP_ACCOUNTS;
use crate::swqos::common::{default_http_client_builder, poll_transaction_confirmation};
use crate::swqos::{SwqosClientTrait, SwqosType, TradeType};

const LABEL: &str = "Apex";
const PING_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone)]
enum ApexBackend {
    Http {
        endpoint: String,
        api_key: String,
        mev_protection: bool,
        http_client: Client,
        _ping: Arc<PingGuard>,
    },
    Quic(ApexSenderClient),
}

struct PingGuard(JoinHandle<()>);

impl Drop for PingGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Clone)]
pub struct ApexClient {
    pub rpc_client: Arc<SolanaRpcClient>,
    backend: ApexBackend,
}

#[async_trait::async_trait]
impl SwqosClientTrait for ApexClient {
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
        let tip_account = *APEX_TIP_ACCOUNTS
            .choose(&mut rand::rng())
            .or_else(|| APEX_TIP_ACCOUNTS.first())
            .unwrap();
        Ok(tip_account.to_string())
    }

    fn get_swqos_type(&self) -> SwqosType {
        SwqosType::Apex
    }
}

impl ApexClient {
    pub fn new_http(
        rpc_url: String,
        endpoint: String,
        api_key: String,
        mev_protection: bool,
    ) -> Result<Self> {
        let endpoint = endpoint.trim_end_matches('/').to_string();
        let http_client = default_http_client_builder().build()?;
        let ping =
            PingGuard(tokio::spawn(ping_loop(http_client.clone(), format!("{endpoint}/ping"))));
        Ok(Self {
            rpc_client: Arc::new(SolanaRpcClient::new(rpc_url)),
            backend: ApexBackend::Http {
                endpoint,
                api_key,
                mev_protection,
                http_client,
                _ping: Arc::new(ping),
            },
        })
    }

    pub async fn new_quic(
        rpc_url: String,
        quic_endpoint: &str,
        api_key: &str,
        mev_protection: bool,
    ) -> Result<Self> {
        let options = ClientOptions {
            endpoint: Some(quic_endpoint.to_string()),
            mev_protect: mev_protection,
            ..ClientOptions::default()
        };
        let quic = ApexSenderClient::connect_with_options(options, api_key).await?;
        Ok(Self {
            rpc_client: Arc::new(SolanaRpcClient::new(rpc_url)),
            backend: ApexBackend::Quic(quic),
        })
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
            .ok_or_else(|| anyhow::anyhow!("Apex transaction has no signature"))?;
        let wire = wincode_serialize(transaction)
            .map_err(|e| anyhow::anyhow!("Apex serialize failed: {e}"))?;

        let submitted = match &self.backend {
            ApexBackend::Http { endpoint, api_key, mev_protection, http_client, .. } => {
                send_http(http_client, endpoint, api_key, *mev_protection, wire).await
            }
            ApexBackend::Quic(quic) => quic
                .send_transaction_bytes(Bytes::from(wire))
                .await
                .map_err(|e| anyhow::anyhow!("Apex QUIC send failed: {e}")),
        };

        if let Err(e) = submitted {
            if crate::common::sdk_log::sdk_log_enabled() {
                crate::common::sdk_log::log_swqos_submission_failed(
                    LABEL,
                    trade_type,
                    start_time.elapsed(),
                    e.to_string(),
                );
            }
            return Err(e);
        }
        if crate::common::sdk_log::sdk_log_enabled() {
            crate::common::sdk_log::log_swqos_submitted(LABEL, trade_type, start_time.elapsed());
        }

        let start_time = Instant::now();
        if let Err(e) =
            poll_transaction_confirmation(&self.rpc_client, signature, wait_confirmation).await
        {
            if crate::common::sdk_log::sdk_log_enabled() {
                println!(" signature: {:?}", signature);
                println!(
                    " [{:width$}] {} confirmation failed: {:?}",
                    LABEL,
                    trade_type,
                    start_time.elapsed(),
                    width = crate::common::sdk_log::SWQOS_LABEL_WIDTH
                );
            }
            return Err(e);
        }
        if wait_confirmation && crate::common::sdk_log::sdk_log_enabled() {
            println!(" signature: {:?}", signature);
            println!(
                " [{:width$}] {} confirmed: {:?}",
                LABEL,
                trade_type,
                start_time.elapsed(),
                width = crate::common::sdk_log::SWQOS_LABEL_WIDTH
            );
        }
        Ok(())
    }
}

async fn ping_loop(http_client: Client, url: String) {
    let mut interval = tokio::time::interval(PING_INTERVAL);
    loop {
        interval.tick().await;
        match http_client.get(&url).timeout(Duration::from_millis(1500)).send().await {
            Ok(response) => {
                let _ = response.bytes().await;
            }
            Err(e) => {
                if crate::common::sdk_log::sdk_log_enabled() {
                    eprintln!("Apex ping request failed: {e}");
                }
            }
        }
    }
}

fn send_bin_url(endpoint: &str, mev_protection: bool) -> String {
    format!("{endpoint}/send-bin?mev_protect={}", u8::from(mev_protection))
}

async fn send_http(
    http_client: &Client,
    endpoint: &str,
    api_key: &str,
    mev_protection: bool,
    wire: Vec<u8>,
) -> Result<()> {
    let response = http_client
        .post(send_bin_url(endpoint, mev_protection))
        .header("x-api-key", api_key)
        .header("Content-Type", "application/octet-stream")
        .body(wire)
        .send()
        .await?;
    let status = response.status();
    let body = response.bytes().await.unwrap_or_default();
    if status.is_success() {
        return Ok(());
    }
    Err(anyhow::anyhow!("Apex send-bin failed: {}", describe_error(status, &body)))
}

fn describe_error(status: reqwest::StatusCode, body: &[u8]) -> String {
    match serde_json::from_slice::<Value>(body) {
        Ok(v) => format!(
            "{status} {}: {}",
            v["error"].as_str().unwrap_or("error"),
            v["message"].as_str().unwrap_or("")
        ),
        Err(_) => format!("{status}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_bin_url_carries_mev_flag() {
        assert_eq!(
            send_bin_url("http://fra.apex.orbitflare.com", false),
            "http://fra.apex.orbitflare.com/send-bin?mev_protect=0"
        );
        assert_eq!(
            send_bin_url("http://fra.apex.orbitflare.com", true),
            "http://fra.apex.orbitflare.com/send-bin?mev_protect=1"
        );
    }

    #[test]
    fn error_body_is_described() {
        let body = br#"{"error":"tip_too_low","message":"tip below the tier floor"}"#;
        assert_eq!(
            describe_error(reqwest::StatusCode::BAD_REQUEST, body),
            "400 Bad Request tip_too_low: tip below the tier floor"
        );
        assert_eq!(describe_error(reqwest::StatusCode::BAD_GATEWAY, b"oops"), "502 Bad Gateway");
    }
}
