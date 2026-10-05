//! Replay saved original wire against the current bank; never signs or sends.
//! cargo run --example simulation_replay -- INPUT.json OUTPUT.json
//! Append --expect-error ComputationalBudgetExceeded for an explicit negative case.
//! This is execution verification only: it does not refresh or re-quote the plan.
use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use sol_trade_sdk::common::SolanaRpcClient;
use solana_client::rpc_config::RpcSimulateTransactionConfig;
use solana_commitment_config::CommitmentConfig;
use solana_rpc_client_api::request::RpcRequest;
use solana_sdk::transaction::VersionedTransaction;
use solana_transaction_status_client_types::UiTransactionEncoding;
use std::io::Write;
use std::time::Duration;

#[derive(Deserialize)]
struct SavedSimulation {
    wire: String,
    #[serde(default)]
    simulation_only_funding: Option<bool>,
}
fn outcome(error: Option<&serde_json::Value>, expected: Option<&str>) -> Result<&'static str> {
    match (error, expected) {
        (None, None) => Ok("execution_succeeded"),
        (Some(error), Some(expected)) if contains_error(error, expected) => {
            Ok("expected_execution_failure")
        }
        (None, Some(_)) => anyhow::bail!("Negative scenario unexpectedly succeeded"),
        (Some(_), _) => anyhow::bail!("Simulation failed; inspect saved response"),
    }
}
// Match exact serialized enum names rather than substrings such as "Budget".
fn contains_error(value: &serde_json::Value, expected: &str) -> bool {
    match value {
        serde_json::Value::String(value) => value == expected,
        serde_json::Value::Array(values) => values.iter().any(|v| contains_error(v, expected)),
        serde_json::Value::Object(values) => {
            values.keys().any(|key| key == expected)
                || values.values().any(|v| contains_error(v, expected))
        }
        _ => false,
    }
}
fn execution_value(result: &serde_json::Value) -> Result<&serde_json::Value> {
    let value = result.get("value").context("Missing simulation value")?;
    ensure!(
        value.as_object().is_some_and(|value| value.contains_key("err")),
        "Missing explicit simulation error status"
    );
    ensure!(
        result
            .get("context")
            .and_then(|context| context.get("slot"))
            .and_then(serde_json::Value::as_u64)
            .is_some(),
        "Missing simulation slot"
    );
    Ok(value)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 2 || (args.len() == 4 && args[2] == "--expect-error" && !args[3].is_empty()),
        "Usage: simulation_replay INPUT.json OUTPUT.json [--expect-error ENUM_NAME]"
    );
    ensure!(
        std::path::Path::new(&args[0]) != std::path::Path::new(&args[1]),
        "Keep original evidence: input and output must be different files"
    );
    ensure!(
        !std::path::Path::new(&args[1]).exists(),
        "Output already exists; select a new evidence path"
    );
    let expected = args.get(3).map(String::as_str);
    let saved: SavedSimulation = serde_json::from_slice(&std::fs::read(&args[0])?)?;
    let wire = STANDARD.decode(&saved.wire).context("Invalid original wire base64")?;
    let transaction: VersionedTransaction =
        wincode::deserialize(&wire).context("Invalid original transaction wire")?;
    ensure!(wincode::serialize(&transaction)? == wire, "Original wire is not canonical");
    let _ = rustls::crypto::ring::default_provider().install_default();
    let rpc = SolanaRpcClient::new_with_commitment(
        std::env::var("RPC_URL").unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".into()),
        CommitmentConfig::confirmed(),
    );
    let config = RpcSimulateTransactionConfig {
        sig_verify: false,
        replace_recent_blockhash: true,
        inner_instructions: true,
        encoding: Some(UiTransactionEncoding::Base64),
        commitment: Some(CommitmentConfig::confirmed()),
        ..Default::default()
    };
    // Keep raw result fields: typed Option<err> cannot distinguish missing from null.
    let result: serde_json::Value = tokio::time::timeout(
        Duration::from_secs(30),
        rpc.send(RpcRequest::SimulateTransaction, serde_json::json!([&saved.wire, config])),
    )
    .await
    .context("Simulation exceeded 30 second deadline")??;
    // Persist both positive and negative responses before deciding the exit status.
    let evidence = serde_json::to_vec_pretty(&serde_json::json!({
        "wire": saved.wire, "response": {"jsonrpc": "2.0", "result": &result},
        "simulation_only_funding": saved.simulation_only_funding, "broadcasts": 0,
        "quote_refreshed": false, "net_credit_verified": false
    }))?;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[1])?
        .write_all(&evidence)?;
    let value = execution_value(&result)?;
    let error = &value["err"];
    let status = outcome(if error.is_null() { None } else { Some(error) }, expected)?;
    println!(
        "{}",
        serde_json::json!({"status":status,"slot":result["context"]["slot"],
        "units_consumed":value.get("unitsConsumed"),"broadcasts":0,"quote_refreshed":false,
        "net_credit_verified":false})
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::outcome;
    use serde_json::json;
    #[test]
    fn unexpected_success_and_unrelated_failures_are_not_accepted() {
        let error = json!({"InstructionError":[0,"ComputationalBudgetExceeded"]});
        assert_eq!(
            outcome(Some(&error), Some("ComputationalBudgetExceeded")).unwrap(),
            "expected_execution_failure"
        );
        assert!(outcome(Some(&error), Some("Budget")).is_err());
        assert!(outcome(None, Some("ComputationalBudgetExceeded")).is_err());
        assert!(outcome(Some(&error), None).is_err());
        assert_eq!(outcome(None, None).unwrap(), "execution_succeeded");
    }
    #[test]
    fn missing_execution_status_is_not_equivalent_to_null() {
        for value in [json!({}), json!(null), json!([])] {
            assert!(super::execution_value(&json!({"context":{"slot":1},"value":value})).is_err());
        }
        assert!(super::execution_value(&json!({"context":{"slot":1},"value":{"err":null}})).is_ok());
        assert!(super::execution_value(&json!({"value":{"err":null}})).is_err());
    }
}
