use base64::{engine::general_purpose::STANDARD, Engine};
use sol_trade_sdk::instruction::token2022_hook::resolve_hook_accounts_with_context;
use solana_sdk::pubkey::Pubkey;
use spl_token_2022_interface::{
    extension::{transfer_hook::get_program_id, StateWithExtensions},
    state::Mint,
};
use std::{collections::HashMap, str::FromStr};
fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/hook_mint_tlv_20261009.json")).unwrap()
}
#[test]
fn official_spl_stops_at_uninitialized_extension() {
    for c in fixture()["cases"].as_array().unwrap() {
        let data = STANDARD.decode(c["mint_data"].as_str().unwrap()).unwrap();
        let mint = StateWithExtensions::<Mint>::unpack(&data).unwrap();
        let program = get_program_id(&mint).map(|p| p.to_string());
        let expected = c["expected_active"]
            .as_bool()
            .unwrap()
            .then(|| c["hook"].as_str().unwrap().to_string());
        assert_eq!(program, expected, "{}", c["name"]);
        println!("{} official_active={}", c["name"], program.is_some());
    }
}
#[test]
fn sdk_hook_resolution_matches_official_tlv_terminator() {
    for c in fixture()["cases"].as_array().unwrap() {
        let pk = |k: &str| Pubkey::from_str(c[k].as_str().unwrap()).unwrap();
        let dec = |k: &str| STANDARD.decode(c[k].as_str().unwrap()).unwrap();
        let keys: Vec<_> = c["execute_accounts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| Pubkey::from_str(p.as_str().unwrap()).unwrap())
            .collect();
        let snapshots: HashMap<_, _> = c["account_data"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| {
                (Pubkey::from_str(k).unwrap(), STANDARD.decode(v.as_str().unwrap()).unwrap())
            })
            .collect();
        let result = resolve_hook_accounts_with_context(
            &pk("hook"),
            &pk("mint"),
            &pk("mint_owner"),
            &dec("mint_data"),
            &pk("meta"),
            &pk("meta_owner"),
            &dec("meta_data"),
            &keys,
            &dec("execute_data"),
            &snapshots,
        );
        if c["expected_active"].as_bool().unwrap() {
            let actual: Vec<_> = result.unwrap().iter().map(|m| m.pubkey.to_string()).collect();
            let expected: Vec<_> = c["expected_accounts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            assert_eq!(actual, expected, "{}", c["name"]);
        } else {
            assert!(result.is_err(), "{} hidden Hook incorrectly activated", c["name"]);
        }
    }
}
