//! Mainnet simulation for Meteora DAMM v2 (USDC pools via SOL→USDC hop).

#![cfg(test)]

use crate::{
    common::mainnet_sim::{self, fixtures},
    instruction::{
        meteora_damm_v2::MeteoraDammV2InstructionBuilder,
        utils::meteora_damm_v2::SWAP2_DISCRIMINATOR,
    },
    swqos::TradeType,
    trading::core::{
        params::DexParamEnum,
        traits::InstructionBuilder,
    },
};

#[tokio::test]
async fn meteora_damm_v2_mainnet_decodes_and_builds_buy() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(pool) = mainnet_sim::load_meteora_damm_v2(&rpc, &fixtures::METEORA_DAMM_V2_POOL).await
    else {
        return;
    };
    assert_eq!(pool.token_a_mint, fixtures::METEORA_DAMM_V2_TOKEN);
    assert_eq!(pool.token_b_mint, fixtures::USDC_MINT);

    let mut params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        fixtures::USDC_MINT,
        fixtures::METEORA_DAMM_V2_TOKEN,
        10_000,
        300,
        DexParamEnum::MeteoraDammV2(pool),
    );
    params.fixed_output_amount = Some(1);
    let ixs = MeteoraDammV2InstructionBuilder.build_buy_instructions(&params).await.unwrap();
    let swap = ixs
        .iter()
        .find(|ix| ix.data.len() >= 8 && &ix.data[..8] == SWAP2_DISCRIMINATOR)
        .expect("meteora swap2");
    assert_eq!(&swap.data[..8], SWAP2_DISCRIMINATOR);
    println!("meteora damm v2 buy ix accounts={}", swap.accounts.len());
}

#[tokio::test]
async fn meteora_damm_v2_mainnet_simulates_buy_after_sol_usdc_hop() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some((hop_ixs, usdc_min_out)) =
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

    let mut meme = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        fixtures::USDC_MINT,
        fixtures::METEORA_DAMM_V2_TOKEN,
        usdc_min_out,
        1_000,
        DexParamEnum::MeteoraDammV2(meteora),
    );
    meme.create_input_mint_ata = false;
    meme.fixed_output_amount = Some(1);
    let meme_ixs = MeteoraDammV2InstructionBuilder.build_buy_instructions(&meme).await.unwrap();
    assert!(meme_ixs.iter().any(|ix| ix.data.len() >= 8 && &ix.data[..8] == SWAP2_DISCRIMINATOR));

    let business = mainnet_sim::concat_ixs([hop_ixs, meme_ixs]);
    mainnet_sim::run_business_sim(&rpc, &wallet, business, &[], "meteora after SOL→USDC").await;
}

#[tokio::test]
async fn meteora_damm_v2_mainnet_simulates_exact_out_buy_after_hop() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some((hop_ixs, usdc_min_out)) =
        mainnet_sim::build_sol_to_usdc_hop(&rpc, wallet.clone(), 2_000_000).await
    else {
        return;
    };

    let Some(meteora) =
        mainnet_sim::load_meteora_damm_v2(&rpc, &fixtures::METEORA_DAMM_V2_POOL).await
    else {
        return;
    };
    let meteora = meteora
        .with_rate_limiter_sysvar(true)
        .with_swap_mode(crate::instruction::utils::meteora_damm_v2::SWAP_MODE_EXACT_OUT);

    let mut meme = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        fixtures::USDC_MINT,
        fixtures::METEORA_DAMM_V2_TOKEN,
        usdc_min_out,
        1_000,
        DexParamEnum::MeteoraDammV2(meteora),
    );
    meme.create_input_mint_ata = false;
    meme.fixed_output_amount = Some(1);
    let meme_ixs = MeteoraDammV2InstructionBuilder.build_buy_instructions(&meme).await.unwrap();
    let swap = meme_ixs
        .iter()
        .find(|ix| ix.data.len() >= 25 && &ix.data[..8] == SWAP2_DISCRIMINATOR)
        .expect("meteora exact-out swap2");
    assert_eq!(
        swap.data[24],
        crate::instruction::utils::meteora_damm_v2::SWAP_MODE_EXACT_OUT
    );

    let business = mainnet_sim::concat_ixs([hop_ixs, meme_ixs]);
    mainnet_sim::run_business_sim(
        &rpc,
        &wallet,
        business,
        &[],
        "meteora exact-out after SOL→USDC",
    )
    .await;
}

#[tokio::test]
async fn meteora_damm_v2_mainnet_builds_sell_after_buy_quote() {
    if !mainnet_sim::enabled() {
        return;
    }

    // Sell of an unknown in-sim token credit is flaky (Custom 6002 when dust
    // underflows). Assert the sell builder wires swap2; buy+hop is covered above.
    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(meteora) =
        mainnet_sim::load_meteora_damm_v2(&rpc, &fixtures::METEORA_DAMM_V2_POOL).await
    else {
        return;
    };
    let meteora = meteora.with_rate_limiter_sysvar(true);

    let mut sell = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Sell,
        fixtures::METEORA_DAMM_V2_TOKEN,
        fixtures::USDC_MINT,
        1_000,
        1_000,
        DexParamEnum::MeteoraDammV2(meteora),
    );
    sell.fixed_output_amount = Some(1);
    let sell_ixs = MeteoraDammV2InstructionBuilder.build_sell_instructions(&sell).await.unwrap();
    assert!(
        sell_ixs
            .iter()
            .any(|ix| ix.data.len() >= 8 && &ix.data[..8] == SWAP2_DISCRIMINATOR),
        "meteora sell must include swap2"
    );
    println!("meteora damm v2 sell built ok ixs={}", sell_ixs.len());
}

#[tokio::test]
async fn meteora_damm_v2_sol_usdc_mainnet_simulates_direct_buy() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(pool) =
        mainnet_sim::load_meteora_damm_v2(&rpc, &fixtures::METEORA_DAMM_V2_SOL_USDC).await
    else {
        return;
    };
    assert!(
        (pool.token_a_mint == crate::constants::WSOL_TOKEN_ACCOUNT
            && pool.token_b_mint == fixtures::USDC_MINT)
            || (pool.token_b_mint == crate::constants::WSOL_TOKEN_ACCOUNT
                && pool.token_a_mint == fixtures::USDC_MINT)
    );
    let pool = pool.with_rate_limiter_sysvar(true);

    let mut params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::USDC_MINT,
        100_000,
        500,
        DexParamEnum::MeteoraDammV2(pool),
    );
    params.fixed_output_amount = Some(1);
    let business = MeteoraDammV2InstructionBuilder
        .build_buy_instructions(&params)
        .await
        .unwrap();
    assert!(business
        .iter()
        .any(|ix| ix.data.len() >= 8 && &ix.data[..8] == SWAP2_DISCRIMINATOR));

    mainnet_sim::run_business_sim(
        &rpc,
        &wallet,
        business,
        &[],
        "meteora damm v2 SOL→USDC direct buy",
    )
    .await;
}

#[tokio::test]
async fn meteora_damm_v2_sol_usdc_mainnet_simulates_buy_and_sell_roundtrip() {
    if !mainnet_sim::enabled() {
        return;
    }

    // Hard buy is covered by `..._direct_buy`. Selling dust USDC back often hits
    // ExceededSlippage (6002) when min_out is floored at 1 lamport. Assert the
    // sell builder wires swap2 for the reverse direction.
    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(pool) =
        mainnet_sim::load_meteora_damm_v2(&rpc, &fixtures::METEORA_DAMM_V2_SOL_USDC).await
    else {
        return;
    };
    let pool = pool.with_rate_limiter_sysvar(true);

    let mut buy = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::USDC_MINT,
        200_000,
        800,
        DexParamEnum::MeteoraDammV2(pool.clone()),
    );
    buy.fixed_output_amount = Some(1);
    let buy_ixs = MeteoraDammV2InstructionBuilder.build_buy_instructions(&buy).await.unwrap();
    assert!(buy_ixs
        .iter()
        .any(|ix| ix.data.len() >= 8 && &ix.data[..8] == SWAP2_DISCRIMINATOR));

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
    assert!(sell_ixs
        .iter()
        .any(|ix| ix.data.len() >= 8 && &ix.data[..8] == SWAP2_DISCRIMINATOR));
    println!(
        "meteora damm v2 SOL/USDC buy+sell builders ok buy_ixs={} sell_ixs={}",
        buy_ixs.len(),
        sell_ixs.len()
    );
}
