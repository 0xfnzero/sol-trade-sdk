use anyhow::Result;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::Value;
use sol_trade_sdk::instruction::{
    token2022_hook::resolve_hook_accounts,
    whirlpool::{swap_v2_with_hooks, WhirlpoolSwapV2Accounts, WhirlpoolSwapV2Args},
};
use solana_sdk::{instruction::AccountMeta, pubkey::Pubkey};
fn pk(v: &Value) -> Pubkey {
    v.as_str().unwrap().parse().unwrap()
}
fn resolved(r: &Value) -> Result<Vec<AccountMeta>> {
    resolve_hook_accounts(
        &pk(&r["hook"]),
        &pk(&r["mint"]),
        &pk(&r["mint_owner"]),
        &STANDARD.decode(r["mint_data"].as_str().unwrap())?,
        &pk(&r["meta"]),
        &pk(&r["meta_owner"]),
        &STANDARD.decode(r["meta_data"].as_str().unwrap())?,
        &r["execute_accounts"].as_array().unwrap().iter().map(pk).collect::<Vec<_>>(),
    )
}
#[test]
fn hook_wires_match_executed_bank() -> Result<()> {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/whirlpool_hook_bank_20261008.json"))?;
    for c in fixture["cases"].as_array().unwrap() {
        let r = &c["resolver"];
        let a = &c["accounts"];
        let accounts = WhirlpoolSwapV2Accounts {
            token_program_a: pk(&a["token_program_a"]),
            token_program_b: pk(&a["token_program_b"]),
            token_authority: pk(&a["token_authority"]),
            whirlpool: pk(&a["whirlpool"]),
            mint_a: pk(&a["mint_a"]),
            mint_b: pk(&a["mint_b"]),
            owner_a: pk(&a["owner_a"]),
            vault_a: pk(&a["vault_a"]),
            owner_b: pk(&a["owner_b"]),
            vault_b: pk(&a["vault_b"]),
            tick_arrays: a["tick_arrays"].as_array().unwrap().iter().map(pk).collect(),
        };
        let extras = resolved(r)?;
        let side_a = accounts.mint_a == pk(&r["mint"]);
        let ix = swap_v2_with_hooks(
            &accounts,
            WhirlpoolSwapV2Args {
                amount: c["amount"].as_u64().unwrap(),
                other_amount_threshold: c["minimum"].as_u64().unwrap(),
                sqrt_price_limit: 0,
                amount_specified_is_input: true,
                a_to_b: c["direction"].as_bool().unwrap(),
            },
            if side_a { &extras } else { &[] },
            if side_a { &[] } else { &extras },
        )?;
        assert_eq!(STANDARD.encode(&ix.data), c["expected"]["data"].as_str().unwrap());
        let metas:Vec<_>=ix.accounts.iter().map(|m|serde_json::json!({"key":m.pubkey.to_string(),"signer":m.is_signer,"writable":m.is_writable})).collect();
        assert_eq!(serde_json::json!(metas), c["expected"]["accounts"]);
    }
    Ok(())
}
#[test]
fn metadata_validation_and_source_dependent_pda() -> Result<()> {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/whirlpool_hook_bank_20261008.json"))?;
    for r in fixture["rewards"].as_array().unwrap() {
        assert_eq!(
            resolved(r)?.iter().map(|m| m.pubkey.to_string()).collect::<Vec<_>>(),
            r["expected_accounts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        );
        for mode in [
            "owner",
            "mint_owner",
            "truncated",
            "count",
            "seed_index",
            "encoding",
            "signer",
            "padding",
        ] {
            let mut bad = r.clone();
            let mut data = STANDARD.decode(r["meta_data"].as_str().unwrap())?;
            match mode {
                "owner" => bad["meta_owner"] = Value::String(Pubkey::default().to_string()),
                "mint_owner" => bad["mint_owner"] = Value::String(Pubkey::default().to_string()),
                "truncated" => {
                    data.pop();
                }
                "count" => data[12] = 255,
                "seed_index" => data[53] = 255,
                "encoding" => data[51] = 2,
                "signer" => data[49] = 1,
                "padding" => data[55] = 3,
                _ => unreachable!(),
            };
            bad["meta_data"] = Value::String(STANDARD.encode(data));
            assert!(resolved(&bad).is_err(), "{mode}");
        }
    }
    Ok(())
}
