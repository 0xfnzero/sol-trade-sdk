//! Mainnet simulation for Raydium AMM v4 (WSOL pairs).

#![cfg(test)]

use crate::{
    common::mainnet_sim::{self, fixtures},
    instruction::{
        raydium_amm_v4::RaydiumAmmV4InstructionBuilder,
        utils::raydium_amm_v4::{SWAP_BASE_IN_V2_DISCRIMINATOR, SWAP_BASE_OUT_V2_DISCRIMINATOR},
    },
    swqos::TradeType,
    trading::core::{
        params::DexParamEnum,
        traits::InstructionBuilder,
    },
};

#[tokio::test]
async fn raydium_amm_v4_mainnet_simulates_buy() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(pool) = mainnet_sim::load_amm_v4(&rpc, fixtures::AMM_V4_WSOL_USDT).await else { return; };
    assert_eq!(pool.coin_mint, crate::constants::WSOL_TOKEN_ACCOUNT);
    assert_eq!(pool.pc_mint, fixtures::USDT_MINT);
    assert!(pool.coin_reserve > 0 && pool.pc_reserve > 0);

    let params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::USDT_MINT,
        100_000,
        300,
        DexParamEnum::RaydiumAmmV4(pool),
    );
    let business = RaydiumAmmV4InstructionBuilder.build_buy_instructions(&params).await.unwrap();
    let swap = business
        .iter()
        .find(|ix| ix.program_id == crate::instruction::utils::raydium_amm_v4::accounts::RAYDIUM_AMM_V4)
        .expect("amm v4 swap ix");
    assert_eq!(&swap.data[..1], SWAP_BASE_IN_V2_DISCRIMINATOR);
    assert!(u64::from_le_bytes(swap.data[9..17].try_into().unwrap()) > 0);

    mainnet_sim::run_business_sim(&rpc, &wallet, business, &[], "amm_v4 wsol→usdt buy").await;
}

#[tokio::test]
async fn raydium_amm_v4_mainnet_simulates_buy_and_sell() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(pool) = mainnet_sim::load_amm_v4(&rpc, fixtures::AMM_V4_WSOL_USDT).await else { return; };

    let buy = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::USDT_MINT,
        100_000,
        300,
        DexParamEnum::RaydiumAmmV4(pool.clone()),
    );
    let buy_ixs = RaydiumAmmV4InstructionBuilder.build_buy_instructions(&buy).await.unwrap();
    let usdt_min_out = {
        let swap = buy_ixs
            .iter()
            .find(|ix| {
                ix.program_id
                    == crate::instruction::utils::raydium_amm_v4::accounts::RAYDIUM_AMM_V4
                    && ix.data.len() >= 17
                    && &ix.data[..1] == SWAP_BASE_IN_V2_DISCRIMINATOR
            })
            .unwrap();
        u64::from_le_bytes(swap.data[9..17].try_into().unwrap())
    };
    let sell_amount = (usdt_min_out / 2).max(1);

    let mut sell = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Sell,
        fixtures::USDT_MINT,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        sell_amount,
        300,
        DexParamEnum::RaydiumAmmV4(pool),
    );
    sell.create_input_mint_ata = false;
    sell.close_input_mint_ata = false;
    sell.create_output_mint_ata = false;
    sell.close_output_mint_ata = true;
    let sell_ixs = RaydiumAmmV4InstructionBuilder.build_sell_instructions(&sell).await.unwrap();

    let mut business = buy_ixs;
    business.extend(sell_ixs);
    mainnet_sim::run_business_sim(&rpc, &wallet, business, &[], "amm_v4 wsol↔usdt buy+sell").await;
}

#[tokio::test]
async fn raydium_amm_v4_mainnet_simulates_exact_out_buy() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(pool) = mainnet_sim::load_amm_v4(&rpc, fixtures::AMM_V4_WSOL_USDT).await else { return; };

    // First quote an exact-in to pick a reachable exact-out target.
    let probe = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::USDT_MINT,
        100_000,
        300,
        DexParamEnum::RaydiumAmmV4(pool.clone()),
    );
    let probe_ixs = RaydiumAmmV4InstructionBuilder.build_buy_instructions(&probe).await.unwrap();
    let min_out = {
        let swap = probe_ixs
            .iter()
            .find(|ix| {
                ix.program_id
                    == crate::instruction::utils::raydium_amm_v4::accounts::RAYDIUM_AMM_V4
                    && ix.data.len() >= 17
                    && &ix.data[..1] == SWAP_BASE_IN_V2_DISCRIMINATOR
            })
            .unwrap();
        u64::from_le_bytes(swap.data[9..17].try_into().unwrap())
    };
    let exact_out = (min_out / 2).max(1);

    let mut params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::USDT_MINT,
        200_000, // max SOL in for exact-out
        300,
        DexParamEnum::RaydiumAmmV4(pool),
    );
    params.fixed_output_amount = Some(exact_out);
    let business = RaydiumAmmV4InstructionBuilder.build_buy_instructions(&params).await.unwrap();
    let swap = business
        .iter()
        .find(|ix| {
            ix.program_id == crate::instruction::utils::raydium_amm_v4::accounts::RAYDIUM_AMM_V4
                && !ix.data.is_empty()
                && &ix.data[..1] == SWAP_BASE_OUT_V2_DISCRIMINATOR
        })
        .expect("exact-out swap");
    assert_eq!(&swap.data[..1], SWAP_BASE_OUT_V2_DISCRIMINATOR);

    mainnet_sim::run_business_sim(&rpc, &wallet, business, &[], "amm_v4 exact-out buy").await;
}

#[tokio::test]
async fn raydium_amm_v4_wsol_usdc_mainnet_simulates_buy() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(pool) = mainnet_sim::load_amm_v4(&rpc, fixtures::AMM_V4_WSOL_USDC).await else { return; };
    assert_eq!(pool.coin_mint, crate::constants::WSOL_TOKEN_ACCOUNT);
    assert_eq!(pool.pc_mint, fixtures::USDC_MINT);

    let params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::USDC_MINT,
        100_000,
        300,
        DexParamEnum::RaydiumAmmV4(pool),
    );
    let business = RaydiumAmmV4InstructionBuilder.build_buy_instructions(&params).await.unwrap();
    let swap = business
        .iter()
        .find(|ix| ix.program_id == crate::instruction::utils::raydium_amm_v4::accounts::RAYDIUM_AMM_V4)
        .expect("amm v4 swap");
    assert_eq!(&swap.data[..1], SWAP_BASE_IN_V2_DISCRIMINATOR);

    mainnet_sim::run_business_sim(&rpc, &wallet, business, &[], "amm_v4 wsol→usdc buy").await;
}

#[tokio::test]
async fn raydium_amm_v4_wsol_usdt_mainnet_simulates_sell_leg_after_buy_probe() {
    if !mainnet_sim::enabled() {
        return;
    }

    // Isolated sell-direction check: buy tiny USDT then sell half back in one sim.
    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(pool) = mainnet_sim::load_amm_v4(&rpc, fixtures::AMM_V4_WSOL_USDT).await else {
        return;
    };

    let buy = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::USDT_MINT,
        50_000,
        500,
        DexParamEnum::RaydiumAmmV4(pool.clone()),
    );
    let buy_ixs = RaydiumAmmV4InstructionBuilder.build_buy_instructions(&buy).await.unwrap();
    let sell_amount = mainnet_sim::amm_v4_min_out(&buy_ixs)
        .map(|v| (v / 2).max(1))
        .unwrap_or(1);

    let mut sell = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Sell,
        fixtures::USDT_MINT,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        sell_amount,
        500,
        DexParamEnum::RaydiumAmmV4(pool),
    );
    sell.create_input_mint_ata = false;
    sell.create_output_mint_ata = false;
    sell.close_output_mint_ata = true;
    let sell_ixs = RaydiumAmmV4InstructionBuilder.build_sell_instructions(&sell).await.unwrap();

    let business = mainnet_sim::concat_ixs([buy_ixs, sell_ixs]);
    mainnet_sim::run_business_sim(
        &rpc,
        &wallet,
        business,
        &[],
        "amm_v4 wsol↔usdt small roundtrip",
    )
    .await;
}

#[tokio::test]
async fn raydium_amm_v4_mainnet_simulates_buy_with_seed_optimize() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(pool) = mainnet_sim::load_amm_v4(&rpc, fixtures::AMM_V4_WSOL_USDT).await else {
        return;
    };

    let params = mainnet_sim::swap_params_seeded(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::USDT_MINT,
        100_000,
        300,
        DexParamEnum::RaydiumAmmV4(pool),
    );
    let business = RaydiumAmmV4InstructionBuilder.build_buy_instructions(&params).await.unwrap();
    mainnet_sim::run_business_sim_skip_oversized(
        &rpc,
        &wallet,
        business,
        &[],
        "amm_v4 wsol→usdt buy (seed-optimize)",
    )
    .await;
}

#[tokio::test]
async fn raydium_amm_v4_mainnet_simulates_exact_out_usdc_buy() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(pool) = mainnet_sim::load_amm_v4(&rpc, fixtures::AMM_V4_WSOL_USDC).await else {
        return;
    };

    let probe = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::USDC_MINT,
        100_000,
        300,
        DexParamEnum::RaydiumAmmV4(pool.clone()),
    );
    let probe_ixs = RaydiumAmmV4InstructionBuilder.build_buy_instructions(&probe).await.unwrap();
    let min_out = mainnet_sim::amm_v4_min_out(&probe_ixs).unwrap_or(1);
    let exact_out = (min_out / 2).max(1);

    let mut params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::USDC_MINT,
        200_000,
        300,
        DexParamEnum::RaydiumAmmV4(pool),
    );
    params.fixed_output_amount = Some(exact_out);
    let business = RaydiumAmmV4InstructionBuilder.build_buy_instructions(&params).await.unwrap();
    let swap = business
        .iter()
        .find(|ix| {
            ix.program_id == crate::instruction::utils::raydium_amm_v4::accounts::RAYDIUM_AMM_V4
                && !ix.data.is_empty()
                && &ix.data[..1] == SWAP_BASE_OUT_V2_DISCRIMINATOR
        })
        .expect("exact-out swap");
    assert_eq!(&swap.data[..1], SWAP_BASE_OUT_V2_DISCRIMINATOR);

    mainnet_sim::run_business_sim(
        &rpc,
        &wallet,
        business,
        &[],
        "amm_v4 wsol→usdc exact-out",
    )
    .await;
}
