use sol_trade_sdk::instruction::{pump_upgrade::build_pump_upgrade_instruction, pump_v3_quote::*};
use solana_sdk::{instruction::AccountMeta, pubkey::Pubkey};
use std::collections::HashMap;
#[test]
fn official_interfaces() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/pump_upgrade/instructions.json")).unwrap();
    for (name, s) in fixture["instructions"].as_object().unwrap() {
        let accounts: HashMap<String, Pubkey> = s["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, a)| {
                (a["name"].as_str().unwrap().to_owned(), Pubkey::new_from_array([i as u8 + 1; 32]))
            })
            .collect();
        let hops: Vec<AccountMeta> = if name == "pump_amm_multi_hop_swap" {
            (0..5)
                .map(|i| {
                    if i < 2 {
                        AccountMeta::new_readonly(Pubkey::new_unique(), false)
                    } else {
                        AccountMeta::new(Pubkey::new_unique(), false)
                    }
                })
                .collect()
        } else {
            vec![]
        };
        let amounts = if s["args"] == 2 { vec![7, 9] } else { vec![] };
        let fill = if s["partial_fill"] == true { Some(true) } else { None };
        let ix = build_pump_upgrade_instruction(name, &accounts, &amounts, fill, &hops).unwrap();
        assert_eq!(ix.program_id.to_string(), s["program"].as_str().unwrap());
        assert_eq!(
            ix.data[..8],
            s["discriminator"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_u64().unwrap() as u8)
                .collect::<Vec<_>>()
        );
        for (idx, a) in s["accounts"].as_array().unwrap().iter().enumerate() {
            let k = &ix.accounts[idx];
            assert_eq!(k.pubkey, accounts[a["name"].as_str().unwrap()]);
            assert_eq!(k.is_writable, a["writable"].as_bool().unwrap_or(false));
            assert_eq!(k.is_signer, a["signer"].as_bool().unwrap_or(false));
        }
        if !amounts.is_empty() {
            assert_eq!(&ix.data[8..16], &7u64.to_le_bytes());
            assert_eq!(&ix.data[16..24], &9u64.to_le_bytes());
        }
        assert!(
            build_pump_upgrade_instruction(name, &HashMap::new(), &amounts, fill, &hops).is_err()
        );
    }
}
#[test]
fn official_quotes() {
    let f: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/pump_upgrade/quotes.json")).unwrap();
    for c in f["cases"].as_array().unwrap() {
        let s = &c["state"];
        let n = |key: &str| s[key].as_str().unwrap().parse::<u64>().unwrap();
        let state = PumpV3QuoteState {
            virtual_base: n("virtual_base"),
            virtual_quote: n("virtual_quote"),
            remaining_base: n("remaining_base"),
            real_quote: n("real_quote"),
            curve_base_balance: n("curve_base_balance"),
            migration_fee: n("migration_fee"),
            protocol_bps: n("protocol_bps"),
            creator_bps: n("creator_bps"),
            ..Default::default()
        };
        let a = c["amount"].as_str().unwrap().parse().unwrap();
        let q = if c["mode"] == "out" {
            quote_pump_buy_v3_exact_out(&state, a, false)
        } else {
            quote_pump_buy_v3_exact_in(&state, a)
        }
        .unwrap();
        assert_eq!(
            if c["mode"] == "out" { q.quote_in } else { q.base_out },
            c["expected"].as_str().unwrap().parse::<u64>().unwrap()
        );
    }
}

#[test]
fn official_create_encoding() {
    use sol_trade_sdk::instruction::pump_create_v2::*;
    let f: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/pump_upgrade/create.json")).unwrap();
    let accounts: HashMap<String, Pubkey> = f["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, a)| {
            (a["name"].as_str().unwrap().to_owned(), Pubkey::new_from_array([i as u8 + 1; 32]))
        })
        .collect();
    let ix = build_pump_create_v2_instruction(
        &accounts,
        PumpCreateV2Params {
            name: "测试",
            symbol: "Q",
            uri: "https://example.com/q",
            creator: Pubkey::new_from_array([9; 32]),
            mayhem: false,
            creator_fee_bps: 25,
            holder_reward: true,
        },
        &[],
    )
    .unwrap();
    let hex: String = ix.data.iter().map(|b| format!("{:02x}", b)).collect();
    assert_eq!(hex, f["data"].as_str().unwrap());
    for (a, m) in f["accounts"].as_array().unwrap().iter().zip(ix.accounts) {
        assert_eq!(m.is_signer, a["signer"].as_bool().unwrap());
        assert_eq!(m.is_writable, a["writable"].as_bool().unwrap());
    }
}
#[test]
fn nested_curve_route() {
    use sol_trade_sdk::instruction::pump_compact_accounts::*;
    let user = Pubkey::new_unique();
    let a = Pubkey::new_unique();
    let b = Pubkey::new_unique();
    let token = solana_sdk::pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
    let wsol = solana_sdk::pubkey!("So11111111111111111111111111111111111111112");
    let hop = |base_mint, quote_mint| {
        let p = derive_pump_v3_accounts(PumpCompactAccountParams {
            user,
            base_mint,
            quote_mint,
            base_token_program: token,
            quote_token_program: token,
            buyback_recipient: user,
            cashback: false,
            complete: false,
        })
        .unwrap();
        PumpMultiHop {
            venue: PumpMultiHopVenue::Curve,
            base_mint,
            quote_mint,
            address: p["bonding_curve"],
            base_vault: p["associated_base_bonding_curve"],
            quote_vault: p["associated_quote_bonding_curve"],
            base_token_program: token,
            quote_token_program: token,
            mayhem: false,
            cashback: false,
            complete: false,
            index: 0,
            creator: Pubkey::default(),
        }
    };
    let mut hops = [hop(a, wsol), hop(b, a)];
    let (accounts, remaining) =
        derive_pump_multi_hop_accounts(user, wsol, b, user, &hops, false).unwrap();
    assert_eq!(accounts.len(), 16);
    assert_eq!(remaining.len(), 10);
    hops[1].cashback = true;
    assert!(derive_pump_multi_hop_accounts(user, wsol, b, user, &hops, false).is_err());
    hops.reverse();
    assert!(derive_pump_multi_hop_accounts(user, wsol, b, user, &hops, false).is_err());
}

#[test]
fn quote_control_and_child_reserves() {
    use sol_trade_sdk::instruction::pump_create_v2::decode_pump_quote_control;
    let f: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/pump_upgrade/quote_control.json")).unwrap();
    let hex = f["data"].as_str().unwrap();
    let data: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    let c = decode_pump_quote_control(&data).unwrap();
    assert_eq!(c.admin.to_string(), f["admin"].as_str().unwrap());
    assert_eq!(c.reserves_admin.to_string(), f["reservesAdmin"].as_str().unwrap());
    assert_eq!(c.mints[0].1, 321);
    assert!(decode_pump_quote_control(&data[..data.len() - 1]).is_err());
    assert_eq!(
        pump_coin_initial_quote_reserves(123, 1000, 10, 1000, 100, 2000, 0, 1).unwrap(),
        12300
    );
    assert!(pump_coin_initial_quote_reserves(123, 1000, 10, 1000, 100, 100, 0, 1).is_err());
}

#[test]
fn builders_match_successful_mainnet_simulations() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/pump_upgrade/simulated_instructions.json"))
            .unwrap();
    for c in fixture["cases"].as_array().unwrap() {
        let accounts = c["accounts"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().parse().unwrap()))
            .collect();
        let amounts: Vec<u64> = c["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap().parse().unwrap())
            .collect();
        let ix = build_pump_upgrade_instruction(
            c["name"].as_str().unwrap(),
            &accounts,
            &amounts,
            None,
            &[],
        )
        .unwrap();
        assert_eq!(ix.program_id.to_string(), c["program"].as_str().unwrap());
        let hex: String = ix.data.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, c["data"].as_str().unwrap());
        for (a, m) in ix.accounts.iter().zip(c["metas"].as_array().unwrap()) {
            assert_eq!(a.pubkey.to_string(), m["pubkey"].as_str().unwrap());
            assert_eq!(a.is_signer, m["signer"].as_bool().unwrap());
            assert_eq!(a.is_writable, m["writable"].as_bool().unwrap());
        }
        assert_eq!(ix.accounts.len(), c["metas"].as_array().unwrap().len());
    }
}
