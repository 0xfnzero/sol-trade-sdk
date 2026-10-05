#[path = "../examples/support/cpmm_creator_fee_simulation.rs"]
mod simulation;

/// Opt-in live RPC test: cargo test --test cpmm_creator_fee_simulation -- --ignored --nocapture
/// Validates real program execution and returned post-state, never broadcasts.
#[tokio::test]
#[ignore = "requires mainnet RPC and a pool with accrued creator fees"]
async fn mainnet_creator_fee_collection_and_legacy_negative_controls() {
    let report = simulation::run_from_env().await.expect("mainnet simulation validation");
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    assert_eq!(report["results"].as_array().unwrap().len(), 2);
    assert!(report["results"].as_array().unwrap().iter().all(|r| r["validated"] == true));
}

#[test]
fn malformed_simulated_accounts_are_not_missing_accounts() {
    use solana_account_decoder::{UiAccount, UiAccountData, UiAccountEncoding};
    let valid = UiAccount {
        lamports: 1,
        data: UiAccountData::Binary("AQ==".into(), UiAccountEncoding::Base64),
        owner: solana_sdk::pubkey::Pubkey::default().to_string(),
        executable: false,
        rent_epoch: 0,
        space: Some(1),
    };
    let decoded = simulation::decode_simulated_accounts(&[None, Some(valid.clone())], 2).unwrap();
    assert!(decoded[0].is_none());
    assert_eq!(decoded[1].as_ref().unwrap().data, vec![1]);
    let mut invalid_data = valid.clone();
    invalid_data.data = UiAccountData::Binary("!invalid!".into(), UiAccountEncoding::Base64);
    assert!(simulation::decode_simulated_accounts(&[Some(invalid_data)], 1).is_err());
    let mut invalid_owner = valid;
    invalid_owner.owner = "invalid pubkey".into();
    assert!(simulation::decode_simulated_accounts(&[Some(invalid_owner)], 1).is_err());
    assert!(simulation::decode_simulated_accounts(&[None], 2).is_err());
}

#[test]
fn snapshot_stability_includes_executable_and_rent_epoch() {
    use solana_sdk::account::Account;
    let original = Account { lamports: 1, ..Account::default() };
    for altered in [
        Account { executable: true, ..original.clone() },
        Account { rent_epoch: 1, ..original.clone() },
    ] {
        assert!(!simulation::unchanged(&[Some(original.clone())], &[Some(altered)]));
    }
    assert!(!simulation::unchanged(
        &[None],
        &[Some(Account { executable: true, ..Account::default() })]
    ));
    assert!(simulation::unchanged(&[None], &[Some(Account::default())]));
}

#[test]
fn token_balance_rejects_truncated_and_uninitialized_state() {
    use solana_sdk::{account::Account, pubkey::Pubkey};
    let mint = Pubkey::new_unique();
    let authority = Pubkey::new_unique();
    let program = sol_trade_sdk::constants::TOKEN_PROGRAM;
    let mut data = vec![0; 165];
    data[..32].copy_from_slice(mint.as_ref());
    data[32..64].copy_from_slice(authority.as_ref());
    data[64..72].copy_from_slice(&42u64.to_le_bytes());
    let account = |data| Account { owner: program, data, ..Account::default() };
    assert!(simulation::token_amount(&account(data[..72].to_vec()), &mint, &authority, &program)
        .is_err());
    assert!(simulation::token_amount(&account(data.clone()), &mint, &authority, &program).is_err());
    data[108] = 1; // SPL AccountState::Initialized
    assert_eq!(simulation::token_amount(&account(data), &mint, &authority, &program).unwrap(), 42);
}

#[test]
fn simulation_requires_explicit_execution_status_and_bank_slot() {
    use serde_json::json;
    let success = json!({"context":{"slot":123},"value":{"err":null,"logs":[]}});
    assert!(simulation::parse_simulation_response(success.clone()).unwrap().value.err.is_none());
    for malformed in [
        json!({"context":{"slot":123},"value":{"logs":[]}}),
        json!({"context":{},"value":{"err":null}}),
        json!({"context":{"slot":-1},"value":{"err":null}}),
        json!({"context":{"slot":"123"},"value":{"err":null}}),
        json!({"context":{"slot":123},"value":{"err":"unexpected"}}),
    ] {
        assert!(simulation::parse_simulation_response(malformed).is_err());
    }
    let failure =
        json!({"context":{"slot":123},"value":{"err":{"InstructionError":[0,{"Custom":3005}]}}});
    let decoded = simulation::parse_simulation_response(failure).unwrap();
    assert_eq!(
        serde_json::to_value(decoded.value.err).unwrap(),
        json!({"InstructionError":[0,{"Custom":3005}]})
    );
}
