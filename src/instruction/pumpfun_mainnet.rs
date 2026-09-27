//! Optional PumpFun bonding-curve mainnet simulation.
//!
//! Live bonding curves rotate quickly. Set `PUMPFUN_MINT=<mint>` to exercise a
//! current incomplete curve; otherwise the suite skips cleanly.

#![cfg(test)]

use crate::{
    common::mainnet_sim,
    instruction::pumpfun::PumpFunInstructionBuilder,
    swqos::TradeType,
    trading::core::{
        params::{DexParamEnum, PumpFunParams},
        traits::InstructionBuilder,
    },
};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

#[tokio::test]
async fn pumpfun_mainnet_simulates_buy_when_mint_env_set() {
    if !mainnet_sim::enabled() {
        return;
    }
    let Ok(mint_str) = std::env::var("PUMPFUN_MINT") else {
        println!("skip pumpfun mainnet sim: set PUMPFUN_MINT to a live bonding-curve mint");
        return;
    };
    let mint = Pubkey::from_str(&mint_str).expect("PUMPFUN_MINT must be a valid pubkey");

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let params_pool =
        PumpFunParams::from_mint_by_rpc(&rpc, &mint).await.expect("decode bonding curve");
    assert!(
        !params_pool.bonding_curve.complete,
        "PUMPFUN_MINT bonding curve is already complete/graduated"
    );
    assert!(params_pool.bonding_curve.virtual_sol_reserves > 0);

    let params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        mint,
        50_000,
        500,
        DexParamEnum::PumpFun(params_pool),
    );
    let business = PumpFunInstructionBuilder.build_buy_instructions(&params).await.unwrap();
    assert!(
        business
            .iter()
            .any(|ix| ix.program_id == crate::instruction::utils::pumpfun::accounts::PUMPFUN),
        "pumpfun buy must include pump program ix"
    );

    mainnet_sim::run_business_sim(&rpc, &wallet, business, &[], "pumpfun buy").await;
}

#[tokio::test]
async fn pumpfun_mainnet_simulates_buy_and_sell_when_mint_env_set() {
    if !mainnet_sim::enabled() {
        return;
    }
    let Ok(mint_str) = std::env::var("PUMPFUN_MINT") else {
        println!("skip pumpfun buy+sell: set PUMPFUN_MINT to a live bonding-curve mint");
        return;
    };
    let mint = Pubkey::from_str(&mint_str).expect("PUMPFUN_MINT must be a valid pubkey");

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let params_pool =
        PumpFunParams::from_mint_by_rpc(&rpc, &mint).await.expect("decode bonding curve");
    assert!(!params_pool.bonding_curve.complete);

    let buy = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        mint,
        50_000,
        500,
        DexParamEnum::PumpFun(params_pool.clone()),
    );
    let buy_ixs = PumpFunInstructionBuilder.build_buy_instructions(&buy).await.unwrap();

    // Conservative dust sell: bonding-curve buy quotes vary; sell 1 raw unit min.
    let sell_amount = 1u64.max(
        buy_ixs
            .iter()
            .find(|ix| ix.program_id == crate::instruction::utils::pumpfun::accounts::PUMPFUN)
            .and_then(|ix| {
                // buy_exact_sol_in stores min_tokens_out near the end of ix data; fall back to 1.
                if ix.data.len() >= 24 {
                    Some((u64::from_le_bytes(ix.data[16..24].try_into().ok()?) / 4).max(1))
                } else {
                    None
                }
            })
            .unwrap_or(1),
    );

    let mut sell = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Sell,
        mint,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        sell_amount,
        800,
        DexParamEnum::PumpFun(params_pool),
    );
    sell.create_input_mint_ata = false;
    sell.create_output_mint_ata = false;
    sell.close_output_mint_ata = true;
    let sell_ixs = PumpFunInstructionBuilder.build_sell_instructions(&sell).await.unwrap();

    let business = mainnet_sim::concat_ixs([buy_ixs, sell_ixs]);
    mainnet_sim::run_business_sim(&rpc, &wallet, business, &[], "pumpfun buy+sell").await;
}
