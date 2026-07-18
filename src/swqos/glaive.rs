use anyhow::{Context as _, Result};
use parking_lot::Mutex;
use rand::seq::IndexedRandom as _;
use reqwest::{Client, StatusCode, Url};
use serde_json::Value;
use solana_sdk::{signature::Signature, transaction::VersionedTransaction};
use std::{
    str::FromStr as _,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::{
    common::SolanaRpcClient,
    constants::swqos::GLAIVE_TIP_ACCOUNTS,
    swqos::{
        common::{default_http_client_builder, poll_transaction_confirmation},
        glaive_quic::GlaiveQuicClient,
        serialization::serialize_transaction_bincode_sync,
        SwqosClientTrait, SwqosType, TradeType,
    },
};

const HTTP_KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(20);

enum GlaiveBackend {
    Http {
        submit_url: Url,
        health_url: Url,
        http_client: Client,
        stop_ping: Arc<AtomicBool>,
        ping_handle: Mutex<Option<JoinHandle<()>>>,
    },
    Quic(Arc<GlaiveQuicClient>),
}

pub struct GlaiveClient {
    rpc_client: Arc<SolanaRpcClient>,
    backend: GlaiveBackend,
}

impl GlaiveClient {
    pub fn new_http(
        rpc_url: String,
        endpoint: String,
        api_key: String,
        mev_protection: bool,
    ) -> Result<Self> {
        validate_api_key(&api_key)?;
        let submit_url = build_binary_url(&endpoint, &api_key, mev_protection)?;
        let health_url = build_health_url(&endpoint)?;
        let http_client = default_http_client_builder().build()?;
        let stop_ping = Arc::new(AtomicBool::new(false));
        let ping_handle = Mutex::new(None);

        let client = Self {
            rpc_client: Arc::new(SolanaRpcClient::new(rpc_url)),
            backend: GlaiveBackend::Http {
                submit_url,
                health_url,
                http_client,
                stop_ping,
                ping_handle,
            },
        };
        client.start_http_keep_alive();
        Ok(client)
    }

    pub async fn new_quic(
        rpc_url: String,
        endpoint: &str,
        api_key: String,
        mev_protection: bool,
    ) -> Result<Self> {
        let quic = GlaiveQuicClient::connect(endpoint, &api_key, mev_protection).await?;
        Ok(Self {
            rpc_client: Arc::new(SolanaRpcClient::new(rpc_url)),
            backend: GlaiveBackend::Quic(Arc::new(quic)),
        })
    }

    fn start_http_keep_alive(&self) {
        let GlaiveBackend::Http { health_url, http_client, stop_ping, ping_handle, .. } =
            &self.backend
        else {
            return;
        };

        let health_url = health_url.clone();
        let http_client = http_client.clone();
        let stop_ping = Arc::clone(stop_ping);
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let handle = runtime.spawn(async move {
            let mut interval = tokio::time::interval(HTTP_KEEP_ALIVE_INTERVAL);
            loop {
                interval.tick().await;
                if stop_ping.load(Ordering::Relaxed) {
                    break;
                }
                if let Err(error) = send_health_request(&http_client, health_url.clone()).await {
                    if crate::common::sdk_log::sdk_log_enabled() {
                        eprintln!(" [Glaive] HTTP keep-alive failed: {error}");
                    }
                }
            }
        });
        *ping_handle.lock() = Some(handle);
    }

    async fn send_transaction_impl(
        &self,
        trade_type: TradeType,
        transaction: &VersionedTransaction,
        wait_confirmation: bool,
    ) -> Result<()> {
        let submit_started = Instant::now();
        let (serialized, signature) = serialize_transaction_bincode_sync(transaction)?;

        let submit_result = match &self.backend {
            GlaiveBackend::Http { submit_url, http_client, .. } => {
                let response = http_client
                    .post(submit_url.clone())
                    .header("Content-Type", "application/octet-stream")
                    .body(serialized.to_vec())
                    .send()
                    .await
                    .map_err(|error| {
                        anyhow::anyhow!("Glaive HTTP request failed: {}", error.without_url())
                    })?;
                let status = response.status();
                let body = response
                    .bytes()
                    .await
                    .map_err(|error| anyhow::anyhow!("read Glaive response: {error}"))?;
                parse_binary_response(status, &body, signature)
            }
            GlaiveBackend::Quic(quic) => quic.send_transaction(&serialized).await,
        };

        if let Err(error) = submit_result {
            if crate::common::sdk_log::sdk_log_enabled() {
                crate::common::sdk_log::log_swqos_submission_failed(
                    "Glaive",
                    trade_type,
                    submit_started.elapsed(),
                    &error,
                );
            }
            return Err(error);
        }
        if crate::common::sdk_log::sdk_log_enabled() {
            crate::common::sdk_log::log_swqos_submitted(
                "Glaive",
                trade_type,
                submit_started.elapsed(),
            );
        }

        let confirm_started = Instant::now();
        if let Err(error) =
            poll_transaction_confirmation(&self.rpc_client, signature, wait_confirmation).await
        {
            if crate::common::sdk_log::sdk_log_enabled() {
                crate::common::sdk_log::log_swqos_submission_failed(
                    "Glaive",
                    trade_type,
                    confirm_started.elapsed(),
                    &error,
                );
            }
            return Err(error);
        }

        if wait_confirmation && crate::common::sdk_log::sdk_log_enabled() {
            println!(" signature: {signature:?}");
            println!(
                " [{:width$}] {} confirmed: {:?}",
                "Glaive",
                trade_type,
                confirm_started.elapsed(),
                width = crate::common::sdk_log::SWQOS_LABEL_WIDTH
            );
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl SwqosClientTrait for GlaiveClient {
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
        let tip_account = *GLAIVE_TIP_ACCOUNTS
            .choose(&mut rand::rng())
            .or_else(|| GLAIVE_TIP_ACCOUNTS.first())
            .context("Glaive tip account list is empty")?;
        Ok(tip_account.to_string())
    }

    fn get_swqos_type(&self) -> SwqosType {
        SwqosType::Glaive
    }
}

impl Drop for GlaiveClient {
    fn drop(&mut self) {
        if let GlaiveBackend::Http { stop_ping, ping_handle, .. } = &self.backend {
            stop_ping.store(true, Ordering::Relaxed);
            if let Some(handle) = ping_handle.lock().take() {
                handle.abort();
            }
        }
    }
}

fn validate_api_key(api_key: &str) -> Result<()> {
    let key = Uuid::parse_str(api_key.trim()).context("Glaive API key must be a valid UUID v4")?;
    if key.get_version_num() != 4 {
        anyhow::bail!("Glaive API key must be a UUID v4");
    }
    Ok(())
}

fn parse_endpoint(endpoint: &str) -> Result<Url> {
    Url::parse(endpoint).context("Glaive HTTP endpoint must be an absolute http(s) URL")
}

fn build_binary_url(endpoint: &str, api_key: &str, mev_protection: bool) -> Result<Url> {
    let mut url = parse_endpoint(endpoint)?;
    if !matches!(url.scheme(), "http" | "https") {
        anyhow::bail!("Glaive HTTP endpoint must use http or https");
    }

    let path = url.path().trim_end_matches('/');
    if !path.ends_with("/binary") {
        let path = if path.is_empty() { "/binary".to_string() } else { format!("{path}/binary") };
        url.set_path(&path);
    }
    let existing_query: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| key != "api-key" && key != "mev-protect")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    url.set_query(None);
    {
        let mut query = url.query_pairs_mut();
        query.extend_pairs(existing_query);
        query.append_pair("api-key", api_key.trim());
        if mev_protection {
            query.append_pair("mev-protect", "true");
        }
    }
    Ok(url)
}

fn build_health_url(endpoint: &str) -> Result<Url> {
    let mut url = parse_endpoint(endpoint)?;
    if !matches!(url.scheme(), "http" | "https") {
        anyhow::bail!("Glaive HTTP endpoint must use http or https");
    }
    url.set_path("/health");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

async fn send_health_request(client: &Client, health_url: Url) -> Result<()> {
    let response =
        client.get(health_url).timeout(Duration::from_millis(1500)).send().await.map_err(
            |error| anyhow::anyhow!("Glaive health request failed: {}", error.without_url()),
        )?;
    let status = response.status();
    response
        .bytes()
        .await
        .map_err(|error| anyhow::anyhow!("read Glaive health response: {}", error.without_url()))?;
    if !status.is_success() {
        anyhow::bail!("Glaive health request returned HTTP {status}");
    }
    Ok(())
}

fn parse_binary_response(status: StatusCode, body: &[u8], expected: Signature) -> Result<()> {
    let json: Value = serde_json::from_slice(body).with_context(|| {
        format!("Glaive returned HTTP {status} with invalid JSON: {}", bounded_body(body))
    })?;

    if let Some(error) = json.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| error.as_str())
            .unwrap_or("unknown Glaive error");
        let code = error.get("code").and_then(Value::as_i64);
        if let Some(code) = code {
            anyhow::bail!("Glaive rejected transaction: code={code} message={message}");
        }
        anyhow::bail!("Glaive rejected transaction: {message}");
    }
    if !status.is_success() {
        anyhow::bail!("Glaive returned HTTP {status}: {}", bounded_body(body));
    }

    let result = json
        .get("result")
        .and_then(Value::as_str)
        .context("Glaive response missing result signature")?;
    let returned = Signature::from_str(result).context("Glaive returned an invalid signature")?;
    if returned != expected {
        anyhow::bail!("Glaive returned a signature that does not match the submitted transaction");
    }
    Ok(())
}

fn bounded_body(body: &[u8]) -> String {
    String::from_utf8_lossy(body).chars().take(512).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_UUID: &str = "00112233-4455-4677-8899-aabbccddeeff";

    #[test]
    fn binary_url_uses_official_query_names() {
        let url = build_binary_url("http://fra.glaive.trade", TEST_UUID, true).unwrap();
        assert_eq!(url.path(), "/binary");
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query.get("api-key").map(String::as_str), Some(TEST_UUID));
        assert_eq!(query.get("mev-protect").map(String::as_str), Some("true"));
        assert!(!query.contains_key("key"));
        assert!(!query.contains_key("mev_protect"));
    }

    #[test]
    fn custom_binary_url_is_not_duplicated_and_health_is_rooted() {
        let url = build_binary_url(
            "https://custom.example/glaive/binary?existing=1&api-key=stale&mev-protect=true",
            TEST_UUID,
            false,
        )
        .unwrap();
        assert_eq!(url.path(), "/glaive/binary");
        assert!(url.query_pairs().any(|(key, value)| key == "existing" && value == "1"));
        assert!(!url.query_pairs().any(|(key, _)| key == "mev-protect"));
        let keys: Vec<_> = url
            .query_pairs()
            .filter(|(key, _)| key == "api-key")
            .map(|(_, value)| value.into_owned())
            .collect();
        assert_eq!(keys, [TEST_UUID]);

        let health = build_health_url("https://custom.example/glaive/binary?existing=1").unwrap();
        assert_eq!(health.as_str(), "https://custom.example/health");
    }

    #[test]
    fn binary_response_requires_matching_signature() {
        let signature = Signature::new_unique();
        let success = format!(r#"{{"result":"{signature}"}}"#);
        assert!(parse_binary_response(StatusCode::OK, success.as_bytes(), signature).is_ok());

        let mismatch = format!(r#"{{"result":"{}"}}"#, Signature::new_unique());
        assert!(parse_binary_response(StatusCode::OK, mismatch.as_bytes(), signature).is_err());
    }

    #[test]
    fn binary_response_surfaces_http_and_rpc_errors() {
        let signature = Signature::new_unique();
        let rpc_error = br#"{"error":{"code":-32000,"message":"missing tip"}}"#;
        let error =
            parse_binary_response(StatusCode::OK, rpc_error, signature).unwrap_err().to_string();
        assert!(error.contains("-32000"));
        assert!(error.contains("missing tip"));

        let http_error = br#"{"error":"rate limit exceeded"}"#;
        let error = parse_binary_response(StatusCode::TOO_MANY_REQUESTS, http_error, signature)
            .unwrap_err()
            .to_string();
        assert!(error.contains("rate limit exceeded"));
    }
}
