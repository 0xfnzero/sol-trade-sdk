use sol_trade_sdk::instruction::pump_upgrade::build_pump_upgrade_instruction;
use solana_sdk::pubkey::Pubkey;
use std::{collections::HashMap, str::FromStr};
#[test]
fn holder_bank_wires() {
    let f: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/pump_holder_bank_20261009.json")).unwrap();
    for c in f["cases"].as_array().unwrap() {
        let roles: HashMap<_, _> = c["roles"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), Pubkey::from_str(v.as_str().unwrap()).unwrap()))
            .collect();
        let args: Vec<u64> = c["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().parse().unwrap())
            .collect();
        let ix = build_pump_upgrade_instruction(
            c["instruction"].as_str().unwrap(),
            &roles,
            &args,
            None,
            &[],
        )
        .unwrap();
        assert_eq!(ix.program_id.to_string(), c["program"]);
        assert_eq!(ix.data.iter().map(|b| format!("{b:02x}")).collect::<String>(), c["data"]);
        assert_eq!(
            ix.accounts.iter().map(|a| a.pubkey.to_string()).collect::<Vec<_>>(),
            c["accounts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|k| k.as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        );
    }
}
