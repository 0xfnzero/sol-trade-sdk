#[path = "../examples/support/cpmm_lp_collection_simulation.rs"]
mod simulation;

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/cpmm_lp_after_collection_mainnet.json")).unwrap()
}

#[tokio::test]
#[ignore = "requires mainnet RPC and a creator with SOL/token1 for simulated LP funding"]
async fn mainnet_lp_is_unchanged_after_creator_fee_collection() {
    let report = simulation::run_from_env().await.expect("mainnet LP comparison");
    assert_eq!(report["comparisons"].as_array().unwrap().len(), 4);
}

#[test]
fn captured_mainnet_lp_balances_supply_fees_events_and_decoders_match() {
    simulation::verify_saved_report(&fixture()).expect("raw mainnet LP fixture validation");
}

#[test]
fn captured_lp_regression_detects_modified_token_balance() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let mut report = fixture();
    let value = &mut report["comparisons"][0]["collected_accounts"][9]["data_base64"];
    let mut bytes = STANDARD.decode(value.as_str().unwrap()).unwrap();
    let amount = u64::from_le_bytes(bytes[64..72].try_into().unwrap());
    bytes[64..72].copy_from_slice(&(amount + 1).to_le_bytes());
    *value = STANDARD.encode(bytes).into();
    let error = simulation::verify_saved_report(&report).unwrap_err();
    assert!(error.to_string().contains("Collection changed user token0"), "{error}");
}

#[test]
fn captured_lp_regression_detects_wrong_protocol_share() {
    let mut report = fixture();
    report["comparisons"][0]["share_rate"] = 0.into();
    let error = simulation::verify_saved_report(&report).unwrap_err();
    assert!(error.to_string().contains("Reported share rate mismatch"), "{error}");
}

#[test]
fn malformed_config_snapshots_return_errors_without_panicking() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    for length in [0, 7, 8, 235] {
        let mut report = fixture();
        let value = &mut report["comparisons"][0]["prepared_accounts"][1]["data_base64"];
        let bytes = STANDARD.decode(value.as_str().unwrap()).unwrap();
        *value = STANDARD.encode(&bytes[..length]).into();
        assert!(simulation::verify_saved_report(&report).is_err());
    }
    let mut report = fixture();
    report["comparisons"][0]["prepared_accounts"][1]["owner"] =
        solana_sdk::pubkey::Pubkey::default().to_string().into();
    assert!(simulation::verify_saved_report(&report).is_err());
}

#[test]
fn reported_execution_delta_must_match_bank_accounts() {
    let mut report = fixture();
    report["comparisons"][0]["token0_delta"] = "0".into();
    let error = simulation::verify_saved_report(&report).unwrap_err();
    assert!(error.to_string().contains("Reported token0_delta"), "{error}");
}
