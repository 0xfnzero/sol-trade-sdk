//! Mainnet simulation for Orca Whirlpool (SOL/USDC).

#![cfg(test)]

use crate::{
    common::mainnet_sim::{self, fixtures},
    instruction::{
        utils::whirlpool::{PROGRAM_ID, SWAP_V2_DISCRIMINATOR},
        whirlpool::WhirlpoolInstructionBuilder,
    },
    swqos::TradeType,
    trading::core::{
        params::DexParamEnum,
        traits::InstructionBuilder,
    },
};

#[tokio::test]
async fn whirlpool_mainnet_simulates_sol_to_usdc_buy() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(pool) = mainnet_sim::load_whirlpool(
        &rpc,
        &fixtures::ORCA_WHIRLPOOL_SOL_USDC,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
        &fixtures::USDC_MINT,
    )
    .await
    else {
        return;
    };
    assert_eq!(pool.mint_a, crate::constants::WSOL_TOKEN_ACCOUNT);
    assert_eq!(pool.mint_b, fixtures::USDC_MINT);
    assert!(pool.tick_arrays.len() >= 3);

    let mut params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::USDC_MINT,
        100_000,
        500,
        DexParamEnum::OrcaWhirlpool(pool),
    );
    params.fixed_output_amount = Some(1);
    let business = WhirlpoolInstructionBuilder
        .build_buy_instructions(&params)
        .await
        .unwrap();
    let swap = business
        .iter()
        .find(|ix| ix.program_id == PROGRAM_ID)
        .expect("whirlpool swap_v2");
    assert_eq!(&swap.data[..8], &SWAP_V2_DISCRIMINATOR);

    mainnet_sim::run_business_sim(&rpc, &wallet, business, &[], "whirlpool sol→usdc buy").await;
}

#[tokio::test]
async fn whirlpool_mainnet_simulates_usdc_to_sol_sell_leg() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some((hop_ixs, usdc_min)) =
        mainnet_sim::build_sol_to_usdc_hop(&rpc, wallet.clone(), 1_000_000).await
    else {
        return;
    };

    let Some(pool) = mainnet_sim::load_whirlpool(
        &rpc,
        &fixtures::ORCA_WHIRLPOOL_SOL_USDC,
        &fixtures::USDC_MINT,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
    )
    .await
    else {
        return;
    };
    let mut sell = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Sell,
        fixtures::USDC_MINT,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        usdc_min,
        800,
        DexParamEnum::OrcaWhirlpool(pool),
    );
    sell.create_input_mint_ata = false;
    sell.create_output_mint_ata = false;
    sell.close_output_mint_ata = true;
    sell.fixed_output_amount = Some(1);
    let sell_ixs = WhirlpoolInstructionBuilder.build_sell_instructions(&sell).await.unwrap();

    let business = mainnet_sim::concat_ixs([hop_ixs, sell_ixs]);
    mainnet_sim::run_business_sim(
        &rpc,
        &wallet,
        business,
        &[],
        "whirlpool after SOL→USDC: usdc→sol",
    )
    .await;
}

#[tokio::test]
async fn whirlpool_mainnet_simulates_buy_and_sell_roundtrip() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(buy_pool) = mainnet_sim::load_whirlpool(
        &rpc,
        &fixtures::ORCA_WHIRLPOOL_SOL_USDC,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
        &fixtures::USDC_MINT,
    )
    .await
    else {
        return;
    };
    let Some(sell_pool) = mainnet_sim::load_whirlpool(
        &rpc,
        &fixtures::ORCA_WHIRLPOOL_SOL_USDC,
        &fixtures::USDC_MINT,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
    )
    .await
    else {
        return;
    };

    let mut buy = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::USDC_MINT,
        200_000,
        800,
        DexParamEnum::OrcaWhirlpool(buy_pool),
    );
    buy.fixed_output_amount = Some(1);
    let buy_ixs = WhirlpoolInstructionBuilder.build_buy_instructions(&buy).await.unwrap();

    let mut sell = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Sell,
        fixtures::USDC_MINT,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        1_000,
        1_000,
        DexParamEnum::OrcaWhirlpool(sell_pool),
    );
    sell.create_input_mint_ata = false;
    sell.create_output_mint_ata = false;
    sell.close_output_mint_ata = true;
    sell.fixed_output_amount = Some(1);
    let sell_ixs = WhirlpoolInstructionBuilder.build_sell_instructions(&sell).await.unwrap();

    let business = mainnet_sim::concat_ixs([buy_ixs, sell_ixs]);
    mainnet_sim::run_business_sim(
        &rpc,
        &wallet,
        business,
        &[],
        "whirlpool sol↔usdc buy+sell",
    )
    .await;
}

#[tokio::test]
async fn whirlpool_sol_usdt_mainnet_simulates_buy() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(pool) = mainnet_sim::load_whirlpool(
        &rpc,
        &fixtures::ORCA_WHIRLPOOL_SOL_USDT,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
        &fixtures::USDT_MINT,
    )
    .await
    else {
        return;
    };
    assert!(
        (pool.mint_a == crate::constants::WSOL_TOKEN_ACCOUNT
            && pool.mint_b == fixtures::USDT_MINT)
            || (pool.mint_b == crate::constants::WSOL_TOKEN_ACCOUNT
                && pool.mint_a == fixtures::USDT_MINT)
    );

    let mut params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::USDT_MINT,
        100_000,
        500,
        DexParamEnum::OrcaWhirlpool(pool),
    );
    params.fixed_output_amount = Some(1);
    let business = WhirlpoolInstructionBuilder
        .build_buy_instructions(&params)
        .await
        .unwrap();
    mainnet_sim::run_business_sim(&rpc, &wallet, business, &[], "whirlpool sol→usdt buy").await;
}

#[tokio::test]
async fn whirlpool_mainnet_simulates_usdt_to_sol_after_hop() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some((hop_ixs, usdt_min)) =
        mainnet_sim::build_sol_to_usdt_hop(&rpc, wallet.clone(), 1_000_000).await
    else {
        return;
    };

    let Some(pool) = mainnet_sim::load_whirlpool(
        &rpc,
        &fixtures::ORCA_WHIRLPOOL_SOL_USDT,
        &fixtures::USDT_MINT,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
    )
    .await
    else {
        return;
    };
    let mut sell = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Sell,
        fixtures::USDT_MINT,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        usdt_min,
        800,
        DexParamEnum::OrcaWhirlpool(pool),
    );
    sell.create_input_mint_ata = false;
    sell.create_output_mint_ata = false;
    sell.close_output_mint_ata = true;
    sell.fixed_output_amount = Some(1);
    let sell_ixs = WhirlpoolInstructionBuilder.build_sell_instructions(&sell).await.unwrap();

    let business = mainnet_sim::concat_ixs([hop_ixs, sell_ixs]);
    mainnet_sim::run_business_sim(
        &rpc,
        &wallet,
        business,
        &[],
        "whirlpool after SOL→USDT: usdt→sol",
    )
    .await;
}
