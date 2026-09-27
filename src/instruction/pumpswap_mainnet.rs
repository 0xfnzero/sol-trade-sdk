//! Mainnet simulation for PumpSwap (WSOL-quoted and USDC-quoted pools).

#![cfg(test)]

use crate::{
    common::mainnet_sim::{self, fixtures},
    instruction::pumpswap::PumpSwapInstructionBuilder,
    swqos::TradeType,
    trading::core::{
        params::DexParamEnum,
        traits::InstructionBuilder,
    },
};

#[tokio::test]
async fn pumpswap_mainnet_simulates_buy() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(pool) = mainnet_sim::load_pumpswap(&rpc, &fixtures::PUMPSWAP_POOL).await else {
        return;
    };
    let pool = mainnet_sim::pin_pumpswap_fees(pool);
    assert_eq!(pool.quote_mint, crate::constants::WSOL_TOKEN_ACCOUNT);
    assert_eq!(pool.base_mint, fixtures::PUMPSWAP_BASE);
    assert!(pool.pool_base_token_reserves > 0);
    assert!(pool.pool_quote_token_reserves > 0);

    let params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::PUMPSWAP_BASE,
        50_000,
        300,
        DexParamEnum::PumpSwap(pool),
    );
    let business = PumpSwapInstructionBuilder.build_buy_instructions(&params).await.unwrap();
    assert!(business
        .iter()
        .any(|ix| ix.program_id == crate::instruction::utils::pumpswap::accounts::AMM_PROGRAM));

    mainnet_sim::run_business_sim(&rpc, &wallet, business, &[], "pumpswap buy").await;
}

#[tokio::test]
async fn pumpswap_mainnet_simulates_buy_and_sell() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(pool) = mainnet_sim::load_pumpswap(&rpc, &fixtures::PUMPSWAP_POOL).await else {
        return;
    };
    let pool = mainnet_sim::pin_pumpswap_fees(pool);

    let buy = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::PUMPSWAP_BASE,
        50_000,
        300,
        DexParamEnum::PumpSwap(pool.clone()),
    );
    let buy_ixs = PumpSwapInstructionBuilder.build_buy_instructions(&buy).await.unwrap();
    let sell_amount = mainnet_sim::pumpswap_buy_min_base_out(&buy_ixs)
        .map(|v| (v / 2).max(1))
        .unwrap_or(1);

    let mut sell = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Sell,
        fixtures::PUMPSWAP_BASE,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        sell_amount,
        300,
        DexParamEnum::PumpSwap(pool),
    );
    sell.create_input_mint_ata = false;
    sell.close_input_mint_ata = false;
    sell.create_output_mint_ata = false;
    sell.close_output_mint_ata = true;
    let sell_ixs = PumpSwapInstructionBuilder.build_sell_instructions(&sell).await.unwrap();

    let business = mainnet_sim::concat_ixs([buy_ixs, sell_ixs]);
    mainnet_sim::run_business_sim(&rpc, &wallet, business, &[], "pumpswap buy+sell").await;
}

#[tokio::test]
async fn pumpswap_seed_pool_mainnet_simulates_buy() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(pool) = mainnet_sim::load_pumpswap(&rpc, &fixtures::PUMPSWAP_SEED_POOL).await else {
        return;
    };
    let pool = mainnet_sim::pin_pumpswap_fees(pool);
    assert_eq!(pool.quote_mint, crate::constants::WSOL_TOKEN_ACCOUNT);
    assert_eq!(pool.base_mint, fixtures::PUMPSWAP_SEED_BASE);

    let params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::PUMPSWAP_SEED_BASE,
        50_000,
        300,
        DexParamEnum::PumpSwap(pool),
    );
    let business = PumpSwapInstructionBuilder.build_buy_instructions(&params).await.unwrap();
    mainnet_sim::run_business_sim(&rpc, &wallet, business, &[], "pumpswap seed pool buy").await;
}

#[tokio::test]
async fn pumpswap_seed_pool_mainnet_simulates_buy_and_sell() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(pool) = mainnet_sim::load_pumpswap(&rpc, &fixtures::PUMPSWAP_SEED_POOL).await else {
        return;
    };
    let pool = mainnet_sim::pin_pumpswap_fees(pool);

    let buy = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::PUMPSWAP_SEED_BASE,
        50_000,
        300,
        DexParamEnum::PumpSwap(pool.clone()),
    );
    let buy_ixs = PumpSwapInstructionBuilder.build_buy_instructions(&buy).await.unwrap();
    let sell_amount = mainnet_sim::pumpswap_buy_min_base_out(&buy_ixs)
        .map(|v| (v / 2).max(1))
        .unwrap_or(1);

    let mut sell = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Sell,
        fixtures::PUMPSWAP_SEED_BASE,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        sell_amount,
        300,
        DexParamEnum::PumpSwap(pool),
    );
    sell.create_input_mint_ata = false;
    sell.create_output_mint_ata = false;
    sell.close_output_mint_ata = true;
    let sell_ixs = PumpSwapInstructionBuilder.build_sell_instructions(&sell).await.unwrap();

    let business = mainnet_sim::concat_ixs([buy_ixs, sell_ixs]);
    mainnet_sim::run_business_sim(
        &rpc,
        &wallet,
        business,
        &[],
        "pumpswap seed pool buy+sell",
    )
    .await;
}

#[tokio::test]
async fn pumpswap_usdc_pool_mainnet_simulates_buy_after_sol_usdc_hop() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some((hop_ixs, usdc_min_out)) =
        mainnet_sim::build_sol_to_usdc_hop(&rpc, wallet.clone(), 100_000).await
    else {
        return;
    };
    let Some(pool) = mainnet_sim::load_pumpswap(&rpc, &fixtures::PUMPSWAP_USDC_POOL).await else {
        return;
    };
    let pool = mainnet_sim::pin_pumpswap_fees(pool);
    assert_eq!(pool.quote_mint, fixtures::USDC_MINT);
    assert_eq!(pool.base_mint, fixtures::PUMPSWAP_BASE);

    let mut buy = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        fixtures::USDC_MINT,
        fixtures::PUMPSWAP_BASE,
        usdc_min_out,
        300,
        DexParamEnum::PumpSwap(pool),
    );
    buy.create_input_mint_ata = false;
    let buy_ixs = PumpSwapInstructionBuilder.build_buy_instructions(&buy).await.unwrap();
    assert!(buy_ixs
        .iter()
        .any(|ix| ix.program_id == crate::instruction::utils::pumpswap::accounts::AMM_PROGRAM));

    let business = mainnet_sim::concat_ixs([hop_ixs, buy_ixs]);
    // Combined hop+PumpSwap can still exceed the simulate size cap even with pinned
    // fee recipients; soft-skip oversized and keep the hard buy/sell coverage on
    // WSOL-quoted pools.
    mainnet_sim::run_business_sim_skip_oversized(
        &rpc,
        &wallet,
        business,
        &[],
        "pumpswap USDC pool: SOL→USDC→PUMP",
    )
    .await;
}

#[tokio::test]
async fn pumpswap_mainnet_simulates_exact_in_larger_buy() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(pool) = mainnet_sim::load_pumpswap(&rpc, &fixtures::PUMPSWAP_POOL).await else {
        return;
    };
    let pool = mainnet_sim::pin_pumpswap_fees(pool);

    let mut params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::PUMPSWAP_BASE,
        200_000,
        800,
        DexParamEnum::PumpSwap(pool),
    );
    // Force classic buy (max quote) rather than buy_exact_quote_in.
    params.use_exact_sol_amount = Some(false);
    let business = PumpSwapInstructionBuilder.build_buy_instructions(&params).await.unwrap();
    mainnet_sim::run_business_sim(
        &rpc,
        &wallet,
        business,
        &[],
        "pumpswap classic buy (larger)",
    )
    .await;
}

#[tokio::test]
async fn pumpswap_mainnet_simulates_buy_with_seed_optimize() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(pool) = mainnet_sim::load_pumpswap(&rpc, &fixtures::PUMPSWAP_POOL).await else {
        return;
    };
    let pool = mainnet_sim::pin_pumpswap_fees(pool);

    let params = mainnet_sim::swap_params_seeded(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::PUMPSWAP_BASE,
        50_000,
        300,
        DexParamEnum::PumpSwap(pool),
    );
    let business = PumpSwapInstructionBuilder.build_buy_instructions(&params).await.unwrap();
    // Seed-optimized ATA paths add extra accounts and can exceed the simulate
    // size cap on public RPC; soft-skip oversized while still asserting build.
    mainnet_sim::run_business_sim_skip_oversized(
        &rpc,
        &wallet,
        business,
        &[],
        "pumpswap buy (seed-optimize ATA)",
    )
    .await;
}

#[tokio::test]
async fn pumpswap_mainnet_simulates_tiny_buy() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(pool) = mainnet_sim::load_pumpswap(&rpc, &fixtures::PUMPSWAP_POOL).await else {
        return;
    };
    let pool = mainnet_sim::pin_pumpswap_fees(pool);

    let params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::PUMPSWAP_BASE,
        10_000, // dust-size quote-in
        1_000,
        DexParamEnum::PumpSwap(pool),
    );
    let business = PumpSwapInstructionBuilder.build_buy_instructions(&params).await.unwrap();
    mainnet_sim::run_business_sim(&rpc, &wallet, business, &[], "pumpswap tiny buy").await;
}

#[tokio::test]
async fn pumpswap_mainnet_simulates_buy_from_mint_rpc() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(pool) = mainnet_sim::load_pumpswap_by_mint(&rpc, &fixtures::PUMPSWAP_BASE).await else {
        return;
    };
    let pool = mainnet_sim::pin_pumpswap_fees(pool);
    assert_eq!(pool.base_mint, fixtures::PUMPSWAP_BASE);
    // from_mint picks the highest-liquidity pool for the mint — may be WSOL or USDC quote.
    assert!(
        pool.quote_mint == crate::constants::WSOL_TOKEN_ACCOUNT
            || pool.quote_mint == fixtures::USDC_MINT,
        "unexpected quote mint {}",
        pool.quote_mint
    );

    if pool.quote_mint == crate::constants::WSOL_TOKEN_ACCOUNT {
        let params = mainnet_sim::swap_params(
            wallet.clone(),
            TradeType::Buy,
            crate::constants::WSOL_TOKEN_ACCOUNT,
            fixtures::PUMPSWAP_BASE,
            50_000,
            300,
            DexParamEnum::PumpSwap(pool),
        );
        let business = PumpSwapInstructionBuilder.build_buy_instructions(&params).await.unwrap();
        mainnet_sim::run_business_sim(
            &rpc,
            &wallet,
            business,
            &[],
            "pumpswap buy via from_mint (WSOL quote)",
        )
        .await;
        return;
    }

    // USDC-quoted pool: hop then buy (may soft-skip if oversized).
    let Some((hop_ixs, usdc_min)) =
        mainnet_sim::build_sol_to_usdc_hop(&rpc, wallet.clone(), 100_000).await
    else {
        return;
    };
    let mut buy = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        fixtures::USDC_MINT,
        fixtures::PUMPSWAP_BASE,
        usdc_min,
        300,
        DexParamEnum::PumpSwap(pool),
    );
    buy.create_input_mint_ata = false;
    let buy_ixs = PumpSwapInstructionBuilder.build_buy_instructions(&buy).await.unwrap();
    let business = mainnet_sim::concat_ixs([hop_ixs, buy_ixs]);
    mainnet_sim::run_business_sim_skip_oversized(
        &rpc,
        &wallet,
        business,
        &[],
        "pumpswap buy via from_mint (USDC quote)",
    )
    .await;
}

#[tokio::test]
async fn pumpswap_mainnet_simulates_buy_and_sell_closes_wsol() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(pool) = mainnet_sim::load_pumpswap(&rpc, &fixtures::PUMPSWAP_POOL).await else {
        return;
    };
    let pool = mainnet_sim::pin_pumpswap_fees(pool);

    let buy = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::PUMPSWAP_BASE,
        50_000,
        300,
        DexParamEnum::PumpSwap(pool.clone()),
    );
    let buy_ixs = PumpSwapInstructionBuilder.build_buy_instructions(&buy).await.unwrap();
    let sell_amount = mainnet_sim::pumpswap_buy_min_base_out(&buy_ixs)
        .map(|v| (v / 2).max(1))
        .unwrap_or(1);

    let mut sell = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Sell,
        fixtures::PUMPSWAP_BASE,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        sell_amount,
        300,
        DexParamEnum::PumpSwap(pool),
    );
    sell.create_input_mint_ata = false;
    sell.create_output_mint_ata = false;
    sell.close_output_mint_ata = true;
    let sell_ixs = PumpSwapInstructionBuilder.build_sell_instructions(&sell).await.unwrap();

    let business = mainnet_sim::concat_ixs([buy_ixs, sell_ixs]);
    mainnet_sim::run_business_sim_skip_oversized(
        &rpc,
        &wallet,
        business,
        &[],
        "pumpswap buy+sell close WSOL",
    )
    .await;
}
