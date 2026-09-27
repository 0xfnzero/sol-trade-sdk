//! Quotes of real mainnet swaps against the output the program credited in
//! `simulateTransaction`, captured by `examples/verify_hop_quotes` with
//! `DUMP_DIR`: static and dynamic fees, every `fee_on` mode, swaps across up to
//! four tick arrays, and limit orders.

use crate::trading::core::params::{ClmmQuoteAccounts, RaydiumClmmParams};
use base64::Engine;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

fn bytes(value: &serde_json::Value) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(value.as_str().unwrap()).unwrap()
}

fn mint(value: &serde_json::Value) -> (Pubkey, Vec<u8>) {
    (Pubkey::from_str(value["owner"].as_str().unwrap()).unwrap(), bytes(&value["data"]))
}

#[test]
fn quotes_match_what_the_program_credited() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/raydium_clmm");
    let mut checked = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let case: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let accounts = &case["accounts"];
        let pool = Pubkey::from_str(case["pool"].as_str().unwrap()).unwrap();
        let (pool_data, config_data) = (bytes(&accounts["pool"]), bytes(&accounts["amm_config"]));
        let bitmap =
            accounts["bitmap_extension"].as_str().map(|_| bytes(&accounts["bitmap_extension"]));
        let (mint_0, mint_1) = (mint(&accounts["token_0_mint"]), mint(&accounts["token_1_mint"]));
        let tick_arrays: Vec<Vec<u8>> =
            accounts["tick_arrays"].as_array().unwrap().iter().map(bytes).collect();
        let mut clock = vec![0u8; 40];
        clock[16..24].copy_from_slice(&case["epoch"].as_u64().unwrap().to_le_bytes());
        clock[32..40].copy_from_slice(&case["unix_timestamp"].as_i64().unwrap().to_le_bytes());

        let params = RaydiumClmmParams::from_quote_accounts(
            pool,
            &ClmmQuoteAccounts {
                pool: &pool_data,
                amm_config: &config_data,
                bitmap_extension: bitmap.as_deref(),
                tick_arrays: tick_arrays.iter().map(Vec::as_slice).collect(),
                token_0_mint: (mint_0.0, &mint_0.1),
                token_1_mint: (mint_1.0, &mint_1.1),
                clock: &clock,
            },
        )
        .unwrap();
        let input = if case["zero_for_one"].as_bool().unwrap() {
            params.token_0_mint
        } else {
            params.token_1_mint
        };
        let quote = params.quote_exact_in(&input, case["amount_in"].as_u64().unwrap()).unwrap();
        assert_eq!(quote.amount_out, case["amount_out"].as_u64().unwrap(), "{}", path.display());
        // The fixture holds exactly the tick arrays the swap crossed.
        assert_eq!(quote.tick_arrays.len(), tick_arrays.len(), "{}", path.display());
        checked += 1;
    }
    assert_eq!(checked, 8);
}
