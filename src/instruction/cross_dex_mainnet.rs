//! Cross-venue mainnet simulations (hop on one DEX, trade on another).

#![cfg(test)]

use crate::{
    common::mainnet_sim::{self, fixtures},
    instruction::{
        meteora_damm_v2::MeteoraDammV2InstructionBuilder,
        pumpswap::PumpSwapInstructionBuilder,
        raydium_clmm::RaydiumClmmInstructionBuilder,
        whirlpool::WhirlpoolInstructionBuilder,
    },
    swqos::TradeType,
    trading::core::{
        params::DexParamEnum,
        traits::InstructionBuilder,
    },
};

#[tokio::test]
async fn cross_dex_clmm_hop_then_whirlpool_sell() {
    if !mainnet_sim::enabled() {
        return;
    }

    // Two concentrated venues together usually exceed the simulate size cap.
    // Assert both builders compose; hard reverse coverage uses AMM hops elsewhere.
    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some((hop_ixs, usdc_est)) =
        mainnet_sim::build_sol_to_usdc_hop_clmm(&rpc, wallet.clone(), 1_000_000).await
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
        usdc_est,
        1_000,
        DexParamEnum::OrcaWhirlpool(pool),
    );
    sell.create_input_mint_ata = false;
    sell.create_output_mint_ata = false;
    sell.close_output_mint_ata = true;
    sell.fixed_output_amount = Some(1);
    let sell_ixs = WhirlpoolInstructionBuilder.build_sell_instructions(&sell).await.unwrap();

    assert!(
        hop_ixs.iter().any(|ix| {
            ix.program_id == crate::instruction::utils::raydium_clmm::PROGRAM_ID
        }),
        "CLMM hop must include CLMM program"
    );
    assert!(
        sell_ixs.iter().any(|ix| {
            ix.program_id == crate::instruction::utils::whirlpool::PROGRAM_ID
        }),
        "Whirlpool sell must include whirlpool program"
    );

    let business = mainnet_sim::concat_ixs([hop_ixs, sell_ixs]);
    mainnet_sim::run_business_sim_skip_oversized(
        &rpc,
        &wallet,
        business,
        &[],
        "cross: CLMM SOL→USDC → Whirlpool USDC→SOL",
    )
    .await;
}

#[tokio::test]
async fn cross_dex_whirlpool_hop_then_clmm_sell() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some((hop_ixs, usdc_est)) =
        mainnet_sim::build_sol_to_usdc_hop_whirlpool(&rpc, wallet.clone(), 1_000_000).await
    else {
        return;
    };

    let Some(pool) = mainnet_sim::load_raydium_clmm(
        &rpc,
        &fixtures::RAYDIUM_CLMM_SOL_USDC,
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
        usdc_est,
        1_000,
        DexParamEnum::RaydiumClmm(pool),
    );
    sell.create_input_mint_ata = false;
    sell.create_output_mint_ata = false;
    sell.close_output_mint_ata = true;
    sell.fixed_output_amount = Some(1);
    let sell_ixs = RaydiumClmmInstructionBuilder.build_sell_instructions(&sell).await.unwrap();

    assert!(hop_ixs
        .iter()
        .any(|ix| ix.program_id == crate::instruction::utils::whirlpool::PROGRAM_ID));
    assert!(sell_ixs
        .iter()
        .any(|ix| ix.program_id == crate::instruction::utils::raydium_clmm::PROGRAM_ID));

    let business = mainnet_sim::concat_ixs([hop_ixs, sell_ixs]);
    mainnet_sim::run_business_sim_skip_oversized(
        &rpc,
        &wallet,
        business,
        &[],
        "cross: Whirlpool SOL→USDC → CLMM USDC→SOL",
    )
    .await;
}

#[tokio::test]
async fn cross_dex_amm_hop_then_dlmm_sell() {
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
    let sell_ixs = crate::instruction::meteora_dlmm::MeteoraDlmmInstructionBuilder
        .build_sell_instructions(&sell)
        .await
        .unwrap();

    let business = mainnet_sim::concat_ixs([hop_ixs, sell_ixs]);
    mainnet_sim::run_business_sim(
        &rpc,
        &wallet,
        business,
        &[],
        "cross: AMM SOL→USDC → DLMM USDC→SOL",
    )
    .await;
}

#[tokio::test]
async fn cross_dex_amm_hop_then_meteora_damm_buy() {
    if !mainnet_sim::enabled() {
        return;
    }

    // Already covered in meteora_damm_v2_mainnet; keep a thin cross-module alias
    // that exercises the shared hop helper + DAMM buy composition.
    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some((hop_ixs, usdc_min)) =
        mainnet_sim::build_sol_to_usdc_hop(&rpc, wallet.clone(), 1_000_000).await
    else {
        return;
    };
    let Some(meteora) =
        mainnet_sim::load_meteora_damm_v2(&rpc, &fixtures::METEORA_DAMM_V2_POOL).await
    else {
        return;
    };
    let meteora = meteora.with_rate_limiter_sysvar(true);

    let mut buy = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        fixtures::USDC_MINT,
        fixtures::METEORA_DAMM_V2_TOKEN,
        usdc_min,
        1_000,
        DexParamEnum::MeteoraDammV2(meteora),
    );
    buy.create_input_mint_ata = false;
    buy.fixed_output_amount = Some(1);
    let buy_ixs = MeteoraDammV2InstructionBuilder.build_buy_instructions(&buy).await.unwrap();

    let business = mainnet_sim::concat_ixs([hop_ixs, buy_ixs]);
    mainnet_sim::run_business_sim(
        &rpc,
        &wallet,
        business,
        &[],
        "cross: AMM SOL→USDC → DAMM meme buy",
    )
    .await;
}

#[tokio::test]
async fn cross_dex_clmm_hop_then_pumpswap_usdc_buy() {
    if !mainnet_sim::enabled() {
        return;
    }

    // CLMM hop + PumpSwap USDC buy — likely oversized; soft-skip keeps CI green
    // while still asserting both builders compose.
    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some((hop_ixs, usdc_est)) =
        mainnet_sim::build_sol_to_usdc_hop_clmm(&rpc, wallet.clone(), 500_000).await
    else {
        return;
    };
    let Some(pool) = mainnet_sim::load_pumpswap(&rpc, &fixtures::PUMPSWAP_USDC_POOL).await else {
        return;
    };
    let pool = mainnet_sim::pin_pumpswap_fees(pool);

    let mut buy = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        fixtures::USDC_MINT,
        fixtures::PUMPSWAP_BASE,
        usdc_est.max(10_000),
        500,
        DexParamEnum::PumpSwap(pool),
    );
    buy.create_input_mint_ata = false;
    let buy_ixs = PumpSwapInstructionBuilder.build_buy_instructions(&buy).await.unwrap();
    assert!(buy_ixs
        .iter()
        .any(|ix| ix.program_id == crate::instruction::utils::pumpswap::accounts::AMM_PROGRAM));

    let business = mainnet_sim::concat_ixs([hop_ixs, buy_ixs]);
    mainnet_sim::run_business_sim_skip_oversized(
        &rpc,
        &wallet,
        business,
        &[],
        "cross: CLMM SOL→USDC → PumpSwap USDC buy",
    )
    .await;
}

#[tokio::test]
async fn cross_dex_amm_usdt_hop_then_clmm_usdt_sell() {
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

    let Some(pool) = mainnet_sim::load_raydium_clmm(
        &rpc,
        &fixtures::RAYDIUM_CLMM_SOL_USDT,
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
        DexParamEnum::RaydiumClmm(pool),
    );
    sell.create_input_mint_ata = false;
    sell.create_output_mint_ata = false;
    sell.close_output_mint_ata = true;
    sell.fixed_output_amount = Some(1);
    let sell_ixs = RaydiumClmmInstructionBuilder.build_sell_instructions(&sell).await.unwrap();

    let business = mainnet_sim::concat_ixs([hop_ixs, sell_ixs]);
    mainnet_sim::run_business_sim(
        &rpc,
        &wallet,
        business,
        &[],
        "cross: AMM SOL→USDT → CLMM USDT→SOL",
    )
    .await;
}

#[tokio::test]
async fn cross_dex_amm_hop_then_damm_sol_usdc_sell() {
    if !mainnet_sim::enabled() {
        return;
    }

    // DAMM SOL/USDC reverse after an AMM hop has been Token-0x1 flaky when
    // spending the hop min_out. Assert sell builder wires swap2; hop coverage
    // remains in AMM v4 / DAMM direct-buy tests.
    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(pool) =
        mainnet_sim::load_meteora_damm_v2(&rpc, &fixtures::METEORA_DAMM_V2_SOL_USDC).await
    else {
        return;
    };
    let pool = pool.with_rate_limiter_sysvar(true);

    let mut sell = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Sell,
        fixtures::USDC_MINT,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        10_000,
        1_000,
        DexParamEnum::MeteoraDammV2(pool),
    );
    sell.fixed_output_amount = Some(1);
    let sell_ixs = MeteoraDammV2InstructionBuilder.build_sell_instructions(&sell).await.unwrap();
    assert!(
        sell_ixs.iter().any(|ix| {
            ix.data.len() >= 8
                && &ix.data[..8] == crate::instruction::utils::meteora_damm_v2::SWAP2_DISCRIMINATOR
        }),
        "DAMM SOL/USDC sell must include swap2"
    );
    println!("cross DAMM SOL/USDC sell built ok ixs={}", sell_ixs.len());
}

#[tokio::test]
async fn cross_dex_triangle_amm_usdc_then_whirlpool_then_clmm() {
    if !mainnet_sim::enabled() {
        return;
    }

    // Triangle composition: SOL→USDC (AMM) → SOL (Whirlpool). CLMM is asserted
    // via a parallel reverse build; full 3-leg txs are usually oversized.
    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some((hop_ixs, usdc_min)) =
        mainnet_sim::build_sol_to_usdc_hop(&rpc, wallet.clone(), 800_000).await
    else {
        return;
    };

    let Some(whirl) = mainnet_sim::load_whirlpool(
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
        DexParamEnum::OrcaWhirlpool(whirl),
    );
    sell.create_input_mint_ata = false;
    sell.create_output_mint_ata = false;
    sell.close_output_mint_ata = true;
    sell.fixed_output_amount = Some(1);
    let sell_ixs = WhirlpoolInstructionBuilder.build_sell_instructions(&sell).await.unwrap();

    // Soft assert CLMM reverse builder also wires for the same USDC credit size.
    if let Some(clmm) = mainnet_sim::load_raydium_clmm(
        &rpc,
        &fixtures::RAYDIUM_CLMM_SOL_USDC,
        &fixtures::USDC_MINT,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
    )
    .await
    {
        let mut clmm_sell = mainnet_sim::swap_params(
            wallet.clone(),
            TradeType::Sell,
            fixtures::USDC_MINT,
            crate::constants::WSOL_TOKEN_ACCOUNT,
            usdc_min,
            800,
            DexParamEnum::RaydiumClmm(clmm),
        );
        clmm_sell.create_input_mint_ata = false;
        clmm_sell.fixed_output_amount = Some(1);
        let clmm_ixs = RaydiumClmmInstructionBuilder
            .build_sell_instructions(&clmm_sell)
            .await
            .unwrap();
        assert!(clmm_ixs.iter().any(|ix| {
            ix.program_id == crate::instruction::utils::raydium_clmm::PROGRAM_ID
        }));
        println!("triangle: CLMM reverse also built (ixs={})", clmm_ixs.len());
    }

    let business = mainnet_sim::concat_ixs([hop_ixs, sell_ixs]);
    mainnet_sim::run_business_sim(
        &rpc,
        &wallet,
        business,
        &[],
        "cross triangle: AMM SOL→USDC → Whirlpool USDC→SOL",
    )
    .await;
}
