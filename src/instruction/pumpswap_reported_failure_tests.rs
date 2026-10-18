use super::*;
use crate::{
    common::mainnet_sim,
    constants::{SOL_TOKEN_ACCOUNT, TOKEN_PROGRAM, TOKEN_PROGRAM_2022, WSOL_TOKEN_ACCOUNT},
    instruction::utils::pumpswap::BUY_DISCRIMINATOR,
    swqos::TradeType,
    trading::core::params::{
        CacheReadContext, CachedAccount, DexParamEnum, PoolTradeHint, SubscriptionAccountCache,
    },
};
use base64::{engine::general_purpose::STANDARD, Engine};

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../../tests/fixtures/pumpswap_reported_failure.json"))
        .unwrap()
}

fn cached_pool(f: &serde_json::Value) -> PumpSwapParams {
    let mut cache = SubscriptionAccountCache::default();
    for a in f["accounts"].as_array().unwrap() {
        cache
            .update(
                a["address"].as_str().unwrap().parse().unwrap(),
                CachedAccount {
                    owner: a["owner"].as_str().unwrap().parse().unwrap(),
                    data: STANDARD.decode(a["data_base64"].as_str().unwrap()).unwrap(),
                    slot: a["slot"].as_u64().unwrap(),
                    write_version: 0,
                },
            )
            .unwrap();
    }
    let key = |i: usize| f["instruction_accounts"][i].as_str().unwrap().parse().unwrap();
    cache
        .pumpswap(
            PoolTradeHint { pool: key(0), input_mint: key(4), output_mint: key(3) },
            CacheReadContext {
                slot: f["current_snapshot_slot"].as_u64().unwrap(),
                epoch: f["epoch"].as_u64().unwrap(),
                maximum_slot_age: 0,
            },
        )
        .unwrap()
}

#[tokio::test]
async fn pumpswap_reported_3007_rejects_missing_creator_accounts_and_rebuilds_buy() {
    let f = fixture();
    assert_eq!(f["error"], serde_json::json!({"InstructionError": [5, {"Custom": 3007}]}));
    let pool = cached_pool(&f);
    assert_eq!(pool.quote_mint, WSOL_TOKEN_ACCOUNT);
    assert_eq!(pool.base_token_program, TOKEN_PROGRAM_2022);
    assert_eq!(pool.quote_token_program, TOKEN_PROGRAM);
    assert_eq!(pool.coin_creator_vault_ata.to_string(), f["correct_creator_vault_ata"]);
    assert_eq!(pool.coin_creator_vault_authority.to_string(), f["correct_creator_vault_authority"]);
    // This independently captured current vault really is a WSOL SPL account.
    let vault =
        STANDARD.decode(f["creator_ata_snapshot"]["data_base64"].as_str().unwrap()).unwrap();
    assert_eq!(f["creator_ata_snapshot"]["owner"], TOKEN_PROGRAM.to_string());
    assert_eq!(&vault[..32], pool.quote_mint.as_ref());
    assert_eq!(&vault[32..64], pool.coin_creator_vault_authority.as_ref());
    let original = bs58::decode(f["instruction_data_base58"].as_str().unwrap()).into_vec().unwrap();
    assert_eq!(&original[..8], BUY_DISCRIMINATOR);
    let mut params = mainnet_sim::swap_params(
        mainnet_sim::create_wallet(),
        TradeType::Buy,
        pool.quote_mint,
        pool.base_mint,
        u64::from_le_bytes(original[16..24].try_into().unwrap()),
        100,
        DexParamEnum::PumpSwap(pool.clone()),
    );
    params.fixed_output_amount = Some(u64::from_le_bytes(original[8..16].try_into().unwrap()));
    let ixs = PumpSwapInstructionBuilder.build_buy_instructions(&params).await.unwrap();
    let ix = ixs.iter().find(|ix| ix.program_id == accounts::AMM_PROGRAM).unwrap();
    // Reproduce the historical Buy payload, while using valid snapshot accounts.
    assert_eq!(ix.data, original);
    for i in [0, 3, 4, 7, 8, 11, 12] {
        assert_eq!(ix.accounts[i].pubkey.to_string(), f["instruction_accounts"][i]);
    }
    assert_eq!(ix.accounts[17].pubkey, pool.coin_creator_vault_ata);
    assert_eq!(ix.accounts[18].pubkey, pool.coin_creator_vault_authority);
    for index in [17, 18] {
        assert_eq!(f["instruction_accounts"][index], Pubkey::default().to_string());
    }
    for (missing_ata, missing_authority) in [(true, false), (false, true), (true, true)] {
        let mut invalid = pool.clone();
        if missing_ata {
            invalid.coin_creator_vault_ata = Pubkey::default();
        }
        if missing_authority {
            invalid.coin_creator_vault_authority = Pubkey::default();
        }
        params.protocol_params = DexParamEnum::PumpSwap(invalid);
        let error = PumpSwapInstructionBuilder.build_buy_instructions(&params).await.unwrap_err();
        assert!(error.to_string().contains("PumpSwap missing coin_creator_vault"), "{error}");
    }
}

#[tokio::test]
async fn pumpswap_reported_cross_language_guards_and_export() {
    let f = fixture();
    let key = |i: usize| f["instruction_accounts"][i].as_str().unwrap().parse::<Pubkey>().unwrap();
    let pool = cached_pool(&f).with_fee_recipients(key(9), key(24));
    let payer = std::sync::Arc::new(solana_sdk::signature::Keypair::new_from_array([7; 32]));
    let mut params = mainnet_sim::swap_params(
        payer.clone(),
        TradeType::Buy,
        pool.quote_mint,
        pool.base_mint,
        1_000_000,
        100,
        DexParamEnum::PumpSwap(pool.clone()),
    );
    params.create_input_mint_ata = false;
    params.create_output_mint_ata = false;
    params.fixed_output_amount = Some(1_000_000);
    for is_mayhem_mode in [false, true] {
        let mut p = pool.clone();
        p.is_mayhem_mode = is_mayhem_mode;
        params.protocol_params = DexParamEnum::PumpSwap(p);
        for is_buy in [true, false] {
            let ixs = if is_buy {
                PumpSwapInstructionBuilder.build_buy_instructions(&params).await
            } else {
                PumpSwapInstructionBuilder.build_sell_instructions(&params).await
            }
            .unwrap();
            assert_eq!(ixs[0].accounts[9].pubkey, key(9));
            assert_eq!(ixs[0].accounts[ixs[0].accounts.len() - 2].pubkey, key(24));
        }
    }
    for invalid in 0..3 {
        let mut p = pool.clone();
        match invalid {
            0 => p.pool_quote_token_account = p.pool_base_token_account,
            1 => p.protocol_fee_recipient_override = Some(Pubkey::default()),
            _ => p.protocol_extra_fee_recipient_override = Some(Pubkey::default()),
        }
        assert!(p.validate_accounts().is_err());
    }
    params.protocol_params = DexParamEnum::PumpSwap(pool);
    let ixs = PumpSwapInstructionBuilder.build_buy_instructions(&params).await.unwrap();
    assert!(!ixs[0].accounts[19].is_writable);
    if let Some(directory) = std::env::var_os("PUMPSWAP_FIXTURE_EXPORT_DIR") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        let ix = &ixs[0];
        let data = serde_json::json!({
            "program": ix.program_id.to_string(), "payer": payer.pubkey().to_string(),
            "data_base64": STANDARD.encode(&ix.data),
            "accounts": ix.accounts.iter().map(|a| serde_json::json!({
                "pubkey": a.pubkey.to_string(), "is_signer": a.is_signer, "is_writable": a.is_writable,
            })).collect::<Vec<_>>(),
        });
        std::fs::write(directory.join("rust.json"), serde_json::to_vec_pretty(&data).unwrap())
            .unwrap();
    }
}

/// Controlled current-state reproduction, not a replay of the old durable nonce
/// or historical price. Both variants use the same wallet, quote and blockhash.
#[tokio::test]
#[ignore = "requires current mainnet state and simulateTransaction"]
async fn pumpswap_reported_3007_mainnet_compares_default_and_correct_creator_accounts() {
    let f = fixture();
    let pool_key = f["instruction_accounts"][0].as_str().unwrap().parse().unwrap();
    let rpc = mainnet_sim::rpc_client();
    let pool = PumpSwapParams::from_pool_address_by_rpc(&rpc, &pool_key).await.unwrap();
    pool.validate_accounts().unwrap();
    assert_eq!(pool.quote_mint, WSOL_TOKEN_ACCOUNT);
    let wallet = mainnet_sim::create_wallet();
    let mut params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        SOL_TOKEN_ACCOUNT,
        pool.base_mint,
        1_000_000,
        500,
        DexParamEnum::PumpSwap(pool.clone()),
    );
    params.use_exact_sol_amount = Some(false); // Original failing instruction was Buy.
    let correct = PumpSwapInstructionBuilder.build_buy_instructions(&params).await.unwrap();
    let mut invalid = correct.clone();
    let ix = invalid.iter_mut().find(|ix| ix.program_id == accounts::AMM_PROGRAM).unwrap();
    assert_eq!(&ix.data[..8], BUY_DISCRIMINATOR);
    ix.accounts[17].pubkey = Pubkey::default();
    ix.accounts[18].pubkey = Pubkey::default();
    let funder = mainnet_sim::pick_funder(&rpc).await;
    let blockhash = rpc.get_latest_blockhash().await.unwrap();
    let bad_tx = mainnet_sim::build_sim_tx(funder, &wallet, invalid, blockhash, &[]);
    let good_tx = mainnet_sim::build_sim_tx(funder, &wallet, correct, blockhash, &[]);
    let bad = mainnet_sim::simulate(&rpc, &bad_tx).await;
    println!("Original default creator accounts: err={:?}, cu={:?}", bad.err, bad.units_consumed);
    assert!(bad.err.as_deref().is_some_and(|e| e.contains("Custom(3007)")));
    assert!(bad
        .logs
        .iter()
        .any(|line| line.contains("coin_creator_vault_ata") && line.contains("3007")));
    let good = mainnet_sim::simulate(&rpc, &good_tx).await;
    println!(
        "Current correct creator ATA={}, authority={}",
        pool.coin_creator_vault_ata, pool.coin_creator_vault_authority
    );
    mainnet_sim::assert_sim_ok("Reported dev/null / WSOL Buy with correct creator accounts", &good);
}
