//! Mainnet simulation for Meteora DLMM (SOL/USDC).

#![cfg(test)]

use crate::{
    common::mainnet_sim::{self, fixtures},
    instruction::{
        meteora_dlmm::MeteoraDlmmInstructionBuilder,
        utils::meteora_dlmm::{PROGRAM_ID, SWAP2_DISCRIMINATOR},
    },
    swqos::TradeType,
    trading::core::{
        params::DexParamEnum,
        traits::InstructionBuilder,
    },
};

#[tokio::test]
async fn meteora_dlmm_mainnet_simulates_sol_to_usdc_buy() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(pool) = mainnet_sim::load_meteora_dlmm(
        &rpc,
        &fixtures::METEORA_DLMM_SOL_USDC,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
        &fixtures::USDC_MINT,
    )
    .await
    else {
        return;
    };
    assert_eq!(pool.token_x_mint, crate::constants::WSOL_TOKEN_ACCOUNT);
    assert_eq!(pool.token_y_mint, fixtures::USDC_MINT);
    assert!(!pool.bin_arrays.is_empty());

    let mut params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::USDC_MINT,
        100_000,
        500,
        DexParamEnum::MeteoraDlmm(pool),
    );
    params.fixed_output_amount = Some(1);
    let business = MeteoraDlmmInstructionBuilder
        .build_buy_instructions(&params)
        .await
        .unwrap();
    let swap = business
        .iter()
        .find(|ix| ix.program_id == PROGRAM_ID)
        .expect("dlmm swap2");
    assert_eq!(&swap.data[..8], &SWAP2_DISCRIMINATOR);

    mainnet_sim::run_business_sim(&rpc, &wallet, business, &[], "dlmm sol→usdc buy").await;
}

#[tokio::test]
async fn meteora_dlmm_mainnet_simulates_usdc_to_sol_after_hop() {
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

    let Some(pool) = mainnet_sim::load_meteora_dlmm(
        &rpc,
        &fixtures::METEORA_DLMM_SOL_USDC,
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
        DexParamEnum::MeteoraDlmm(pool),
    );
    sell.create_input_mint_ata = false;
    sell.create_output_mint_ata = false;
    sell.close_output_mint_ata = true;
    sell.fixed_output_amount = Some(1);
    let sell_ixs = MeteoraDlmmInstructionBuilder.build_sell_instructions(&sell).await.unwrap();

    let business = mainnet_sim::concat_ixs([hop_ixs, sell_ixs]);
    mainnet_sim::run_business_sim(
        &rpc,
        &wallet,
        business,
        &[],
        "dlmm after SOL→USDC: usdc→sol",
    )
    .await;
}

#[tokio::test]
async fn meteora_dlmm_mainnet_simulates_buy_and_sell_roundtrip() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(buy_pool) = mainnet_sim::load_meteora_dlmm(
        &rpc,
        &fixtures::METEORA_DLMM_SOL_USDC,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
        &fixtures::USDC_MINT,
    )
    .await
    else {
        return;
    };
    let Some(sell_pool) = mainnet_sim::load_meteora_dlmm(
        &rpc,
        &fixtures::METEORA_DLMM_SOL_USDC,
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
        DexParamEnum::MeteoraDlmm(buy_pool),
    );
    buy.fixed_output_amount = Some(1);
    let buy_ixs = MeteoraDlmmInstructionBuilder.build_buy_instructions(&buy).await.unwrap();

    let mut sell = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Sell,
        fixtures::USDC_MINT,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        1_000,
        1_000,
        DexParamEnum::MeteoraDlmm(sell_pool),
    );
    sell.create_input_mint_ata = false;
    sell.create_output_mint_ata = false;
    sell.close_output_mint_ata = true;
    sell.fixed_output_amount = Some(1);
    let sell_ixs = MeteoraDlmmInstructionBuilder.build_sell_instructions(&sell).await.unwrap();

    let business = mainnet_sim::concat_ixs([buy_ixs, sell_ixs]);
    mainnet_sim::run_business_sim(&rpc, &wallet, business, &[], "dlmm sol↔usdc buy+sell").await;
}

#[tokio::test]
async fn meteora_dlmm_alt_pool_mainnet_simulates_sol_to_usdc_buy() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(pool) = mainnet_sim::load_meteora_dlmm(
        &rpc,
        &fixtures::METEORA_DLMM_SOL_USDC_ALT,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
        &fixtures::USDC_MINT,
    )
    .await
    else {
        return;
    };
    assert!(
        (pool.token_x_mint == crate::constants::WSOL_TOKEN_ACCOUNT
            && pool.token_y_mint == fixtures::USDC_MINT)
            || (pool.token_y_mint == crate::constants::WSOL_TOKEN_ACCOUNT
                && pool.token_x_mint == fixtures::USDC_MINT)
    );
    assert!(!pool.bin_arrays.is_empty());

    let mut params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::USDC_MINT,
        100_000,
        500,
        DexParamEnum::MeteoraDlmm(pool),
    );
    params.fixed_output_amount = Some(1);
    let business = MeteoraDlmmInstructionBuilder
        .build_buy_instructions(&params)
        .await
        .unwrap();
    mainnet_sim::run_business_sim(
        &rpc,
        &wallet,
        business,
        &[],
        "dlmm alt-pool sol→usdc buy",
    )
    .await;
}

#[tokio::test]
async fn meteora_dlmm_alt_pool_mainnet_simulates_usdc_to_sol_after_hop() {
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

    let Some(pool) = mainnet_sim::load_meteora_dlmm(
        &rpc,
        &fixtures::METEORA_DLMM_SOL_USDC_ALT,
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
        DexParamEnum::MeteoraDlmm(pool),
    );
    sell.create_input_mint_ata = false;
    sell.create_output_mint_ata = false;
    sell.close_output_mint_ata = true;
    sell.fixed_output_amount = Some(1);
    let sell_ixs = MeteoraDlmmInstructionBuilder.build_sell_instructions(&sell).await.unwrap();

    let business = mainnet_sim::concat_ixs([hop_ixs, sell_ixs]);
    mainnet_sim::run_business_sim(
        &rpc,
        &wallet,
        business,
        &[],
        "dlmm alt-pool after SOL→USDC: usdc→sol",
    )
    .await;
}
