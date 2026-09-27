//! Mainnet simulation for Raydium CPMM pools used by StonkFun SOL hops.

#![cfg(test)]

use crate::{
    common::mainnet_sim::{self, fixtures},
    instruction::{
        raydium_cpmm::RaydiumCpmmInstructionBuilder,
        utils::raydium_cpmm::{
            accounts as cpmm_accounts, SWAP_BASE_IN_DISCRIMINATOR, SWAP_BASE_OUT_DISCRIMINATOR,
        },
    },
    swqos::TradeType,
    trading::core::{
        params::DexParamEnum,
        traits::InstructionBuilder,
    },
};

#[tokio::test]
async fn raydium_cpmm_wsol_stonk_mainnet_simulates_buy() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let pool =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::WSOL_STONK_CPMM).await else { return; }; v };
    assert!(pool.base_reserve > 0 && pool.quote_reserve > 0);

    let params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::GRAD_QUOTE_STONK,
        50_000,
        300,
        DexParamEnum::RaydiumCpmm(pool),
    );
    let business = RaydiumCpmmInstructionBuilder.build_buy_instructions(&params).await.unwrap();
    let swap = business
        .iter()
        .find(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM)
        .expect("cpmm swap ix");
    assert_eq!(&swap.data[..8], SWAP_BASE_IN_DISCRIMINATOR);
    assert!(u64::from_le_bytes(swap.data[16..24].try_into().unwrap()) > 0);

    let alt = mainnet_sim::load_alt(&rpc, &fixtures::WSOL_STONK_LUT).await;
    mainnet_sim::run_business_sim(&rpc, &wallet, business, &[alt], "cpmm wsol→stonk buy").await;
}

#[tokio::test]
async fn raydium_cpmm_wsol_stonk_mainnet_simulates_buy_and_sell() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let pool =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::WSOL_STONK_CPMM).await else { return; }; v };

    let buy = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::GRAD_QUOTE_STONK,
        50_000,
        300,
        DexParamEnum::RaydiumCpmm(pool.clone()),
    );
    let buy_ixs = RaydiumCpmmInstructionBuilder.build_buy_instructions(&buy).await.unwrap();
    let stonk_min_out = {
        let swap = buy_ixs
            .iter()
            .find(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM)
            .unwrap();
        u64::from_le_bytes(swap.data[16..24].try_into().unwrap())
    };
    let sell_amount = (stonk_min_out / 2).max(1);

    let mut sell = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Sell,
        fixtures::GRAD_QUOTE_STONK,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        sell_amount,
        300,
        DexParamEnum::RaydiumCpmm(pool),
    );
    sell.create_input_mint_ata = false;
    sell.close_input_mint_ata = false;
    sell.create_output_mint_ata = false;
    sell.close_output_mint_ata = false;
    let sell_ixs = RaydiumCpmmInstructionBuilder.build_sell_instructions(&sell).await.unwrap();

    let mut business = buy_ixs;
    business.extend(sell_ixs);
    let alt = mainnet_sim::load_alt(&rpc, &fixtures::WSOL_STONK_LUT).await;
    mainnet_sim::run_business_sim(&rpc, &wallet, business, &[alt], "cpmm wsol↔stonk buy+sell")
        .await;
}

#[tokio::test]
async fn raydium_cpmm_wsol_cards_mainnet_simulates_buy() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let pool =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::WSOL_CARDS_CPMM).await else { return; }; v };

    let params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::CURVE_QUOTE_CARDS,
        20_000,
        500,
        DexParamEnum::RaydiumCpmm(pool),
    );
    let business = RaydiumCpmmInstructionBuilder.build_buy_instructions(&params).await.unwrap();
    assert!(business.iter().any(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM));

    let alt = mainnet_sim::load_alt(&rpc, &fixtures::WSOL_CARDS_LUT).await;
    mainnet_sim::run_business_sim(&rpc, &wallet, business, &[alt], "cpmm wsol→cards buy").await;
}

#[tokio::test]
async fn raydium_cpmm_wsol_cards_mainnet_simulates_buy_and_sell() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let pool =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::WSOL_CARDS_CPMM).await else { return; }; v };

    let buy = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::CURVE_QUOTE_CARDS,
        20_000,
        500,
        DexParamEnum::RaydiumCpmm(pool.clone()),
    );
    let buy_ixs = RaydiumCpmmInstructionBuilder.build_buy_instructions(&buy).await.unwrap();
    let cards_min_out = {
        let swap = buy_ixs
            .iter()
            .find(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM)
            .unwrap();
        u64::from_le_bytes(swap.data[16..24].try_into().unwrap())
    };
    let sell_amount = (cards_min_out / 2).max(1);

    let mut sell = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Sell,
        fixtures::CURVE_QUOTE_CARDS,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        sell_amount,
        500,
        DexParamEnum::RaydiumCpmm(pool),
    );
    sell.create_input_mint_ata = false;
    sell.close_input_mint_ata = false;
    sell.create_output_mint_ata = false;
    sell.close_output_mint_ata = false;
    let sell_ixs = RaydiumCpmmInstructionBuilder.build_sell_instructions(&sell).await.unwrap();

    let mut business = buy_ixs;
    business.extend(sell_ixs);
    let alt = mainnet_sim::load_alt(&rpc, &fixtures::WSOL_CARDS_LUT).await;
    mainnet_sim::run_business_sim(&rpc, &wallet, business, &[alt], "cpmm wsol↔cards buy+sell")
        .await;
}

#[tokio::test]
async fn raydium_cpmm_graduated_knots_stonk_mainnet_simulates_both_directions_after_sol_hop() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let graduated =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::GRAD_POOL).await else { return; }; v };
    let sol_hop =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::WSOL_STONK_CPMM).await else { return; }; v };

    // Acquire STONK, then swap STONK ↔ KNOTS both ways on the graduated pool
    // using the plain RaydiumCpmm builder (not StonkFunSwap wrapper).
    let hop = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::GRAD_QUOTE_STONK,
        50_000,
        300,
        DexParamEnum::RaydiumCpmm(sol_hop),
    );
    let hop_ixs = RaydiumCpmmInstructionBuilder.build_buy_instructions(&hop).await.unwrap();
    let stonk_min_out = {
        let swap = hop_ixs
            .iter()
            .find(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM)
            .unwrap();
        u64::from_le_bytes(swap.data[16..24].try_into().unwrap())
    };

    let mut buy_meme = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        fixtures::GRAD_QUOTE_STONK,
        fixtures::GRAD_MEME_KNOTS,
        stonk_min_out,
        300,
        DexParamEnum::RaydiumCpmm(graduated.clone()),
    );
    buy_meme.create_input_mint_ata = false;
    let buy_ixs =
        RaydiumCpmmInstructionBuilder.build_buy_instructions(&buy_meme).await.unwrap();
    let meme_min_out = {
        let swap = buy_ixs
            .iter()
            .rev()
            .find(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM)
            .unwrap();
        u64::from_le_bytes(swap.data[16..24].try_into().unwrap())
    };

    let mut sell_meme = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Sell,
        fixtures::GRAD_MEME_KNOTS,
        fixtures::GRAD_QUOTE_STONK,
        (meme_min_out / 2).max(1),
        300,
        DexParamEnum::RaydiumCpmm(graduated),
    );
    sell_meme.create_input_mint_ata = false;
    sell_meme.close_input_mint_ata = false;
    sell_meme.create_output_mint_ata = false;
    sell_meme.close_output_mint_ata = false;
    let sell_ixs =
        RaydiumCpmmInstructionBuilder.build_sell_instructions(&sell_meme).await.unwrap();

    let mut business = hop_ixs;
    business.extend(buy_ixs);
    business.extend(sell_ixs);
    let alt = mainnet_sim::load_alt(&rpc, &fixtures::WSOL_STONK_LUT).await;
    mainnet_sim::run_business_sim(
        &rpc,
        &wallet,
        business,
        &[alt],
        "cpmm graduated knots/stonk both dirs",
    )
    .await;
}

#[tokio::test]
async fn raydium_cpmm_wsol_stonk_mainnet_simulates_exact_out_buy() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let pool =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::WSOL_STONK_CPMM).await else { return; }; v };

    let probe = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::GRAD_QUOTE_STONK,
        50_000,
        300,
        DexParamEnum::RaydiumCpmm(pool.clone()),
    );
    let probe_ixs = RaydiumCpmmInstructionBuilder.build_buy_instructions(&probe).await.unwrap();
    let min_out = {
        let swap = probe_ixs
            .iter()
            .find(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM)
            .unwrap();
        u64::from_le_bytes(swap.data[16..24].try_into().unwrap())
    };
    let exact_out = (min_out / 2).max(1);

    let mut params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::GRAD_QUOTE_STONK,
        100_000,
        300,
        DexParamEnum::RaydiumCpmm(pool),
    );
    params.fixed_output_amount = Some(exact_out);
    let business = RaydiumCpmmInstructionBuilder.build_buy_instructions(&params).await.unwrap();
    let swap = business
        .iter()
        .find(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM)
        .unwrap();
    assert_eq!(&swap.data[..8], SWAP_BASE_OUT_DISCRIMINATOR);

    let alt = mainnet_sim::load_alt(&rpc, &fixtures::WSOL_STONK_LUT).await;
    mainnet_sim::run_business_sim(&rpc, &wallet, business, &[alt], "cpmm wsol→stonk exact-out")
        .await;
}

#[tokio::test]
async fn raydium_cpmm_wsol_cards_mainnet_simulates_exact_out_buy() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let Some(pool) = mainnet_sim::load_cpmm(&rpc, &fixtures::WSOL_CARDS_CPMM).await else {
        return;
    };

    let probe = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::CURVE_QUOTE_CARDS,
        50_000,
        300,
        DexParamEnum::RaydiumCpmm(pool.clone()),
    );
    let probe_ixs = RaydiumCpmmInstructionBuilder.build_buy_instructions(&probe).await.unwrap();
    let min_out = mainnet_sim::cpmm_min_out(&probe_ixs).unwrap_or(1);
    let exact_out = (min_out / 2).max(1);

    let mut params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::CURVE_QUOTE_CARDS,
        100_000,
        300,
        DexParamEnum::RaydiumCpmm(pool),
    );
    params.fixed_output_amount = Some(exact_out);
    let business = RaydiumCpmmInstructionBuilder.build_buy_instructions(&params).await.unwrap();
    let swap = business
        .iter()
        .find(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM)
        .unwrap();
    assert_eq!(&swap.data[..8], SWAP_BASE_OUT_DISCRIMINATOR);

    let alt = mainnet_sim::load_alt(&rpc, &fixtures::WSOL_CARDS_LUT).await;
    mainnet_sim::run_business_sim(&rpc, &wallet, business, &[alt], "cpmm wsol→cards exact-out")
        .await;
}
