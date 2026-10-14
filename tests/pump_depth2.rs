use base64::{engine::general_purpose::STANDARD, Engine};
use sol_trade_sdk::instruction::{pump_compact_accounts::*, pump_create_v2::*};
use solana_sdk::{pubkey::Pubkey, transaction::VersionedTransaction};
use std::{collections::HashMap, str::FromStr};
fn key(v: &serde_json::Value) -> Pubkey {
    Pubkey::from_str(v.as_str().unwrap()).unwrap()
}
fn hop(v: &serde_json::Value) -> PumpMultiHop {
    PumpMultiHop {
        venue: if v["venue"] == "pool" {
            PumpMultiHopVenue::Pool
        } else {
            PumpMultiHopVenue::Curve
        },
        base_mint: key(&v["baseMint"]),
        quote_mint: key(&v["quoteMint"]),
        address: key(&v["address"]),
        base_vault: key(&v["baseVault"]),
        quote_vault: key(&v["quoteVault"]),
        base_token_program: key(&v["baseTokenProgram"]),
        quote_token_program: key(&v["quoteTokenProgram"]),
        mayhem: false,
        cashback: false,
        complete: v["complete"].as_bool().unwrap(),
        index: 0,
        creator: key(&v["creator"]),
    }
}
#[test]
fn official_depth2_accounts_and_signed_wire() {
    let f: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/pump_upgrade/create_depth2.json")).unwrap();
    for c in f["cases"].as_array().unwrap() {
        let accounts: HashMap<String, Pubkey> =
            c["accounts"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), key(v))).collect();
        let q = hop(&c["quote"]);
        let chain: Vec<_> = c["chain"].as_array().unwrap().iter().map(hop).collect();
        let listed: Vec<_> = c["listedQuoteMints"].as_array().unwrap().iter().map(key).collect();
        let remaining = derive_pump_coin_quote_create_accounts_with_chain(
            accounts["mint"],
            q,
            &chain,
            1,
            2,
            &listed,
        )
        .unwrap();
        // Later whitelist additions do not rewrite Q's stored depth or chain.
        let mut expanded_listed = listed.clone();
        expanded_listed.push(chain[0].base_mint);
        assert_eq!(
            derive_pump_coin_quote_create_accounts_with_chain(
                accounts["mint"],
                q,
                &chain,
                1,
                2,
                &expanded_listed
            )
            .unwrap(),
            remaining
        );
        let p = &c["params"];
        let ix = build_pump_create_v2_instruction(
            &accounts,
            PumpCreateV2Params {
                name: p["name"].as_str().unwrap(),
                symbol: p["symbol"].as_str().unwrap(),
                uri: p["uri"].as_str().unwrap(),
                creator: key(&p["creator"]),
                mayhem: false,
                creator_fee_bps: 25,
                holder_reward: true,
            },
            &remaining,
        )
        .unwrap();
        let expected_data = c["data"].as_str().unwrap();
        let actual: String = ix.data.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(actual, expected_data);
        let expected = c["keys"].as_array().unwrap();
        assert_eq!(ix.accounts.len(), expected.len());
        for (m, e) in ix.accounts.iter().zip(expected) {
            assert_eq!(m.pubkey, key(&e["pubkey"]));
            assert_eq!(m.is_signer, e["signer"].as_bool().unwrap());
            assert_eq!(m.is_writable, e["writable"].as_bool().unwrap());
        }
        let wire = STANDARD.decode(c["transaction"].as_str().unwrap()).unwrap();
        let tx: VersionedTransaction = bincode::deserialize(&wire).unwrap();
        tx.verify_and_hash_message().unwrap();
        assert_eq!(bincode::serialize(&tx).unwrap(), wire);
        assert!(derive_pump_coin_quote_create_accounts(accounts["mint"], q, 1, 2, &listed).is_err());
        assert!(derive_pump_coin_quote_create_accounts_with_chain(
            accounts["mint"],
            q,
            &chain,
            1,
            1,
            &listed
        )
        .is_err());
        let mut bad = chain.clone();
        let mut bad_quote = q;
        bad_quote.mayhem = true;
        assert!(derive_pump_coin_quote_create_accounts_with_chain(
            accounts["mint"],
            bad_quote,
            &chain,
            1,
            2,
            &listed
        )
        .is_err());
        // Creation prohibits mayhem on Q, but not on its ancestor.
        bad[0].mayhem = true;
        assert_eq!(
            derive_pump_coin_quote_create_accounts_with_chain(
                accounts["mint"],
                q,
                &bad,
                1,
                2,
                &listed
            )
            .unwrap(),
            remaining
        );
        bad[0] = PumpMultiHop { base_mint: accounts["user"], ..chain[0] };
        assert!(derive_pump_coin_quote_create_accounts_with_chain(
            accounts["mint"],
            q,
            &bad,
            1,
            2,
            &listed
        )
        .unwrap_err()
        .to_string()
        .contains("discontinuous quote chain"));
        bad[0] = PumpMultiHop { base_token_program: accounts["program"], ..chain[0] };
        assert!(derive_pump_coin_quote_create_accounts_with_chain(
            accounts["mint"],
            q,
            &bad,
            1,
            2,
            &listed
        )
        .unwrap_err()
        .to_string()
        .contains("discontinuous quote chain"));
        assert!(derive_pump_coin_quote_create_accounts_with_chain(
            accounts["mint"],
            q,
            &chain,
            5,
            255,
            &listed
        )
        .unwrap_err()
        .to_string()
        .contains("CurveDepthExceeded"));
        for (index, signer, writable) in [(2, false, true), (4, true, false)] {
            let mut invalid = remaining.clone();
            invalid[index].is_signer = signer;
            invalid[index].is_writable = writable;
            assert!(build_pump_create_v2_instruction(
                &accounts,
                PumpCreateV2Params {
                    name: "x",
                    symbol: "x",
                    uri: "x",
                    creator: accounts["user"],
                    mayhem: false,
                    creator_fee_bps: 0,
                    holder_reward: false,
                },
                &invalid
            )
            .is_err());
        }
        bad[0] = chain[0];
        bad[0].base_mint = accounts["mint"];
        assert!(derive_pump_coin_quote_create_accounts_with_chain(
            accounts["mint"],
            q,
            &bad,
            1,
            2,
            &listed
        )
        .is_err());
    }
}
