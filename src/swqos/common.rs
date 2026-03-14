use crate::common::types::SolanaRpcClient;
use crate::swqos::serialization;
use anyhow::Result;
use base64::engine::general_purpose::{self, STANDARD};
use base64::Engine;
use bincode::serialize;
use reqwest::Client;
use serde_json;
use serde_json::json;
use solana_client::rpc_client::SerializableTransaction;
use solana_client::rpc_config::RpcTransactionConfig;
use solana_sdk::signature::Signature;
use solana_sdk::transaction::VersionedTransaction;
use solana_sdk::transaction::{Transaction, TransactionError};
use solana_transaction_status::{TransactionConfirmationStatus, UiTransactionEncoding};
use std::str::FromStr;
use std::time::{Duration, Instant};
use tokio::time::sleep;

/// Default pool idle timeout for SWQOS HTTP client (seconds). 连接池空闲超时（秒）。
const HTTP_POOL_IDLE_TIMEOUT_SECS: u64 = 300;
/// Max idle connections per host. 每主机最大空闲连接数。
const HTTP_POOL_MAX_IDLE_PER_HOST: usize = 4;
/// TCP keepalive interval (seconds). TCP 保活间隔（秒）。
const HTTP_TCP_KEEPALIVE_SECS: u64 = 60;
/// HTTP/2 keepalive interval (seconds). HTTP/2 保活间隔（秒）。
const HTTP2_KEEPALIVE_INTERVAL_SECS: u64 = 10;
/// HTTP/2 keepalive timeout (seconds). HTTP/2 保活超时（秒）。
const HTTP2_KEEPALIVE_TIMEOUT_SECS: u64 = 5;
/// Request timeout (milliseconds). 请求超时（毫秒）。
const HTTP_TIMEOUT_MS: u64 = 3000;
/// Connect timeout (milliseconds). 连接超时（毫秒）。
const HTTP_CONNECT_TIMEOUT_MS: u64 = 2000;

/// Shared HTTP client builder for SWQOS clients; call `.build().unwrap()` or override pool first. SWQOS 共用 HTTP 客户端构建器。
pub fn default_http_client_builder() -> reqwest::ClientBuilder {
    Client::builder()
        .pool_idle_timeout(Duration::from_secs(HTTP_POOL_IDLE_TIMEOUT_SECS))
        .pool_max_idle_per_host(HTTP_POOL_MAX_IDLE_PER_HOST)
        .tcp_keepalive(Some(Duration::from_secs(HTTP_TCP_KEEPALIVE_SECS)))
        .tcp_nodelay(true)
        .http2_keep_alive_interval(Duration::from_secs(HTTP2_KEEPALIVE_INTERVAL_SECS))
        .http2_keep_alive_timeout(Duration::from_secs(HTTP2_KEEPALIVE_TIMEOUT_SECS))
        .http2_adaptive_window(true)
        .timeout(Duration::from_millis(HTTP_TIMEOUT_MS))
        .connect_timeout(Duration::from_millis(HTTP_CONNECT_TIMEOUT_MS))
}

/// Trade/on-chain error with code and optional instruction index. 交易/链上错误，含错误码与可选指令下标。
#[derive(Debug, Clone)]
pub struct TradeError {
    pub code: u32,
    pub message: String,
    pub instruction: Option<u8>,
}

impl std::fmt::Display for TradeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for TradeError {}

impl From<anyhow::Error> for TradeError {
    fn from(e: anyhow::Error) -> Self {
        if let Some(te) = e.downcast_ref::<TradeError>() {
            return te.clone();
        }
        TradeError { code: 500, message: format!("{}", e), instruction: None }
    }
}

// High-performance serialization

pub trait FormatBase64VersionedTransaction {
    fn to_base64_string(&self) -> String;
}

impl FormatBase64VersionedTransaction for VersionedTransaction {
    fn to_base64_string(&self) -> String {
        let tx_bytes = bincode::serialize(self).unwrap();
        general_purpose::STANDARD.encode(tx_bytes)
    }
}

pub async fn poll_transaction_confirmation(
    rpc: &SolanaRpcClient,
    txt_sig: Signature,
    wait_confirmation: bool,
) -> Result<Signature> {
    poll_any_transaction_confirmation(rpc, &[txt_sig], wait_confirmation).await
}

/// Poll multiple signatures in parallel (one RPC call per poll) and return the first one that confirms.
/// When transactions are submitted to multiple SWQOS channels, each channel produces a different
/// signature. Only one will land on-chain, so we must check all of them.
pub async fn poll_any_transaction_confirmation(
    rpc: &SolanaRpcClient,
    signatures: &[Signature],
    wait_confirmation: bool,
) -> Result<Signature> {
    if signatures.is_empty() {
        return Err(anyhow::anyhow!("No signatures to confirm"));
    }
    // If no confirmation needed, return first signature immediately
    if !wait_confirmation {
        return Ok(signatures[0]);
    }

    let timeout: Duration = Duration::from_secs(15);
    let interval: Duration = Duration::from_millis(1000);
    let start: Instant = Instant::now();
    let mut poll_count = 0u32;
    // Track which signature landed (confirmed or failed on-chain)
    let mut landed_sig: Option<Signature> = None;

    loop {
        if start.elapsed() >= timeout {
            return Err(anyhow::anyhow!(
                "Transaction confirmation timed out after {}s ({} signatures polled)",
                timeout.as_secs(),
                signatures.len()
            ));
        }

        poll_count += 1;

        let status = rpc.get_signature_statuses(signatures).await?;
        // Check all signatures for any that confirmed successfully
        for (i, maybe_status) in status.value.iter().enumerate() {
            if let Some(s) = maybe_status {
                if s.err.is_none()
                    && (s.confirmation_status == Some(TransactionConfirmationStatus::Confirmed)
                        || s.confirmation_status == Some(TransactionConfirmationStatus::Finalized))
                {
                    return Ok(signatures[i]);
                }
                // Track the first signature that landed on-chain (even if errored)
                if landed_sig.is_none() {
                    landed_sig = Some(signatures[i]);
                }
            }
        }

        // If no signature has any status yet, keep waiting
        if landed_sig.is_none() {
            sleep(interval).await;
            continue;
        }

        let landed = landed_sig.unwrap();
        let should_get_transaction = poll_count >= 10;

        if !should_get_transaction {
            sleep(interval).await;
            continue;
        }

        let tx_details = match rpc
            .get_transaction_with_config(
                &landed,
                RpcTransactionConfig {
                    encoding: Some(UiTransactionEncoding::JsonParsed),
                    max_supported_transaction_version: Some(0),
                    commitment: Some(solana_commitment_config::CommitmentConfig::confirmed()),
                },
            )
            .await
        {
            Ok(details) => details,
            Err(_) => {
                // Tx may not be on chain yet, keep waiting
                sleep(interval).await;
                continue;
            }
        };

        let meta = tx_details.transaction.meta;
        if meta.is_none() {
            sleep(interval).await;
        } else {
            let meta = meta.unwrap();
            if meta.err.is_none() {
                return Ok(landed);
            } else {
                // Extract error message from log_messages
                let mut error_msg = String::new();
                if let solana_transaction_status::option_serializer::OptionSerializer::Some(logs) =
                    &meta.log_messages
                {
                    for log in logs {
                        if let Some(idx) = log.find("Error Message: ") {
                            let msg = log[idx + 15..].trim_end_matches('.').to_string();
                            if !error_msg.is_empty() {
                                error_msg.push_str("; ");
                            }
                            error_msg.push_str(&msg);
                        } else if let Some(idx) = log.find("Program log: Error: ") {
                            let msg = log[idx + 20..].trim_end_matches('.').to_string();
                            if !error_msg.is_empty() {
                                error_msg.push_str("; ");
                            }
                            error_msg.push_str(&msg);
                        }
                    }
                }

                let ui_err = meta.err.unwrap();
                let tx_err: TransactionError =
                    serde_json::from_value(serde_json::to_value(&ui_err)?)?;

                // Use Solana InstructionError codes directly
                let mut code = 0u32;
                let mut index = None;
                match &tx_err {
                    TransactionError::InstructionError(i, i_error) => {
                        // Match all InstructionError variants including Custom
                        code = match i_error {
                            solana_sdk::instruction::InstructionError::Custom(c) => *c,
                            solana_sdk::instruction::InstructionError::GenericError => 1,
                            solana_sdk::instruction::InstructionError::InvalidArgument => 2,
                            solana_sdk::instruction::InstructionError::InvalidInstructionData => 3,
                            solana_sdk::instruction::InstructionError::InvalidAccountData => 4,
                            solana_sdk::instruction::InstructionError::AccountDataTooSmall => 5,
                            solana_sdk::instruction::InstructionError::InsufficientFunds => 6,
                            solana_sdk::instruction::InstructionError::IncorrectProgramId => 7,
                            solana_sdk::instruction::InstructionError::MissingRequiredSignature => 8,
                            solana_sdk::instruction::InstructionError::AccountAlreadyInitialized => 9,
                            solana_sdk::instruction::InstructionError::UninitializedAccount => 10,
                            _ => 999, // Other unknown errors
                        };
                        index = Some(*i);
                    }
                    _ => {}
                }

                return Err(anyhow::Error::new(TradeError {
                    code: code,
                    message: format!("{} {:?}", tx_err, error_msg),
                    instruction: index,
                }));
            }
        }
    }
}

pub async fn send_nb_transaction(
    client: Client,
    endpoint: &str,
    auth_token: &str,
    transaction: &Transaction,
) -> Result<Signature, anyhow::Error> {
    // Serialize transaction
    let serialized = bincode::serialize(transaction)
        .map_err(|e| anyhow::anyhow!("Transaction serialization failed: {}", e))?;

    // Base64 encode
    let encoded = STANDARD.encode(serialized);

    let request_data = json!({
        "transaction": {
            "content": encoded
        },
        "frontRunningProtection": true
    });

    let url = format!("{}/api/v2/submit", endpoint);
    let response = client
        .post(url)
        .header("Authorization", auth_token)
        .header("Content-Type", "application/json")
        .json(&request_data)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Request failed: {}", e))?;

    let resp = response
        .json::<serde_json::Value>()
        .await
        .map_err(|e| anyhow::anyhow!("Response parsing failed: {}", e))?;

    if let Some(reason) = resp["reason"].as_str() {
        return Err(anyhow::anyhow!(reason.to_string()));
    }

    let signature = resp["signature"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing signature field in response"))?;

    let signature =
        Signature::from_str(signature).map_err(|e| anyhow::anyhow!("Invalid signature: {}", e))?;

    Ok(signature)
}

pub async fn serialize_and_encode(
    transaction: &Vec<u8>,
    encoding: UiTransactionEncoding,
) -> Result<String> {
    let serialized = match encoding {
        UiTransactionEncoding::Base58 => bs58::encode(transaction).into_string(),
        UiTransactionEncoding::Base64 => STANDARD.encode(transaction),
        _ => return Err(anyhow::anyhow!("Unsupported encoding")),
    };
    Ok(serialized)
}

/// Sync serialize and encode; uses buffer pool when possible for lower allocs and latency.
pub fn serialize_transaction_and_encode(
    transaction: &impl SerializableTransaction,
    encoding: UiTransactionEncoding,
) -> Result<(String, Signature)> {
    serialization::serialize_transaction_sync(transaction, encoding)
}

pub async fn serialize_smart_transaction_and_encode(
    transaction: &impl SerializableTransaction,
    encoding: UiTransactionEncoding,
) -> Result<(String, Signature)> {
    let signature = transaction.get_signature();
    let serialized_tx = serialize(transaction)?;
    let serialized = match encoding {
        UiTransactionEncoding::Base58 => bs58::encode(serialized_tx).into_string(),
        UiTransactionEncoding::Base64 => STANDARD.encode(serialized_tx),
        _ => return Err(anyhow::anyhow!("Unsupported encoding")),
    };
    Ok((serialized, *signature))
}
