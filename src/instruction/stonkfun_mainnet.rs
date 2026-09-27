//! Mainnet simulation for direct StonkFun curve + graduated CPMM (quote ↔ meme).
//!
//! Ephemeral wallets start with SOL only, so each case first hops SOL → quote on
//! a live Raydium CPMM pool, then runs the StonkFun leg — all inside one
//! `simulateTransaction`.

#![cfg(test)]

use crate::{
    common::mainnet_sim::{self, fixtures},
    instruction::{
        stonkfun::StonkFunInstructionBuilder,
        utils::raydium_cpmm::{accounts as cpmm_accounts, SWAP_BASE_IN_DISCRIMINATOR},
    },
    swqos::TradeType,
    trading::core::{
        params::DexParamEnum,
        traits::InstructionBuilder,
    },
};

#[tokio::test]
async fn stonkfun_curve_mainnet_simulates_quote_buy_after_sol_hop() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let curve = { let Some(v) = mainnet_sim::load_stonkfun_curve(&rpc, &fixtures::CURVE_POOL).await else { return; }; v };
    let sol_hop =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::WSOL_CARDS_CPMM).await else { return; }; v };
    assert_eq!(curve.quote_mint, fixtures::CURVE_QUOTE_CARDS);

    // Hop 1: SOL → CARDS
    let hop1 = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::CURVE_QUOTE_CARDS,
        20_000,
        500,
        DexParamEnum::RaydiumCpmm(sol_hop),
    );
    let hop1_ixs =
        crate::instruction::raydium_cpmm::RaydiumCpmmInstructionBuilder
            .build_buy_instructions(&hop1)
            .await
            .unwrap();
    let hop1_swap = hop1_ixs
        .iter()
        .find(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM)
        .expect("sol→cards swap");
    assert_eq!(&hop1_swap.data[..8], SWAP_BASE_IN_DISCRIMINATOR);
    let cards_min_out = u64::from_le_bytes(hop1_swap.data[16..24].try_into().unwrap());
    assert!(cards_min_out > 0);

    // Hop 2: CARDS → meme on LaunchLab curve
    let mut hop2 = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        fixtures::CURVE_QUOTE_CARDS,
        fixtures::CURVE_MEME,
        cards_min_out,
        500,
        DexParamEnum::StonkFun(curve),
    );
    hop2.create_input_mint_ata = false;
    hop2.close_input_mint_ata = false;
    let hop2_ixs = StonkFunInstructionBuilder.build_buy_instructions(&hop2).await.unwrap();
    assert!(hop2_ixs
        .iter()
        .any(|ix| ix.program_id == crate::instruction::utils::bonk::accounts::BONK));

    let mut business = hop1_ixs;
    business.extend(hop2_ixs);
    let alt = mainnet_sim::load_alt(&rpc, &fixtures::WSOL_CARDS_LUT).await;
    mainnet_sim::run_business_sim(&rpc, &wallet, business, &[alt], "stonkfun curve quote buy")
        .await;
}

#[tokio::test]
async fn stonkfun_curve_mainnet_simulates_buy_and_sell_roundtrip() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let curve = { let Some(v) = mainnet_sim::load_stonkfun_curve(&rpc, &fixtures::CURVE_POOL).await else { return; }; v };
    let sol_hop =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::WSOL_CARDS_CPMM).await else { return; }; v };

    let hop1 = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::CURVE_QUOTE_CARDS,
        20_000,
        500,
        DexParamEnum::RaydiumCpmm(sol_hop),
    );
    let hop1_ixs =
        crate::instruction::raydium_cpmm::RaydiumCpmmInstructionBuilder
            .build_buy_instructions(&hop1)
            .await
            .unwrap();
    let cards_min_out = {
        let swap = hop1_ixs
            .iter()
            .find(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM)
            .unwrap();
        u64::from_le_bytes(swap.data[16..24].try_into().unwrap())
    };

    let mut buy = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        fixtures::CURVE_QUOTE_CARDS,
        fixtures::CURVE_MEME,
        cards_min_out,
        500,
        DexParamEnum::StonkFun(curve.clone()),
    );
    buy.create_input_mint_ata = false;
    let buy_ixs = StonkFunInstructionBuilder.build_buy_instructions(&buy).await.unwrap();
    let meme_min_out = {
        let ix = buy_ixs
            .iter()
            .find(|ix| ix.program_id == crate::instruction::utils::bonk::accounts::BONK)
            .unwrap();
        u64::from_le_bytes(ix.data[16..24].try_into().unwrap())
    };
    let sell_amount = (meme_min_out / 2).max(1);

    let mut sell = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Sell,
        fixtures::CURVE_MEME,
        fixtures::CURVE_QUOTE_CARDS,
        sell_amount,
        500,
        DexParamEnum::StonkFun(curve),
    );
    sell.create_input_mint_ata = false;
    sell.close_input_mint_ata = false;
    sell.create_output_mint_ata = false;
    sell.close_output_mint_ata = false;
    let sell_ixs = StonkFunInstructionBuilder.build_sell_instructions(&sell).await.unwrap();

    let mut business = hop1_ixs;
    business.extend(buy_ixs);
    business.extend(sell_ixs);
    let alt = mainnet_sim::load_alt(&rpc, &fixtures::WSOL_CARDS_LUT).await;
    mainnet_sim::run_business_sim(
        &rpc,
        &wallet,
        business,
        &[alt],
        "stonkfun curve buy+sell roundtrip",
    )
    .await;
}

#[tokio::test]
async fn stonkfun_swap_mainnet_simulates_graduated_buy_after_sol_hop() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let graduated =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::GRAD_POOL).await else { return; }; v };
    let sol_hop =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::WSOL_STONK_CPMM).await else { return; }; v };

    // Hop 1: SOL → STONK
    let hop1 = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::GRAD_QUOTE_STONK,
        50_000,
        300,
        DexParamEnum::RaydiumCpmm(sol_hop),
    );
    let hop1_ixs =
        crate::instruction::raydium_cpmm::RaydiumCpmmInstructionBuilder
            .build_buy_instructions(&hop1)
            .await
            .unwrap();
    let stonk_min_out = {
        let swap = hop1_ixs
            .iter()
            .find(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM)
            .unwrap();
        u64::from_le_bytes(swap.data[16..24].try_into().unwrap())
    };
    assert!(stonk_min_out > 0);

    // Hop 2: STONK → KNOTS via StonkFunSwap
    let mut hop2 = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        fixtures::GRAD_QUOTE_STONK,
        fixtures::GRAD_MEME_KNOTS,
        stonk_min_out,
        300,
        DexParamEnum::StonkFunSwap(graduated),
    );
    hop2.create_input_mint_ata = false;
    let hop2_ixs = StonkFunInstructionBuilder.build_buy_instructions(&hop2).await.unwrap();
    assert!(hop2_ixs.iter().any(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM));

    let mut business = hop1_ixs;
    business.extend(hop2_ixs);
    let alt = mainnet_sim::load_alt(&rpc, &fixtures::WSOL_STONK_LUT).await;
    mainnet_sim::run_business_sim(
        &rpc,
        &wallet,
        business,
        &[alt],
        "stonkfun swap graduated buy",
    )
    .await;
}

#[tokio::test]
async fn stonkfun_swap_mainnet_simulates_graduated_buy_and_sell_roundtrip() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let graduated =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::GRAD_POOL).await else { return; }; v };
    let sol_hop =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::WSOL_STONK_CPMM).await else { return; }; v };

    let hop1 = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::GRAD_QUOTE_STONK,
        50_000,
        300,
        DexParamEnum::RaydiumCpmm(sol_hop),
    );
    let hop1_ixs =
        crate::instruction::raydium_cpmm::RaydiumCpmmInstructionBuilder
            .build_buy_instructions(&hop1)
            .await
            .unwrap();
    let stonk_min_out = {
        let swap = hop1_ixs
            .iter()
            .find(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM)
            .unwrap();
        u64::from_le_bytes(swap.data[16..24].try_into().unwrap())
    };

    let mut buy = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        fixtures::GRAD_QUOTE_STONK,
        fixtures::GRAD_MEME_KNOTS,
        stonk_min_out,
        300,
        DexParamEnum::StonkFunSwap(graduated.clone()),
    );
    buy.create_input_mint_ata = false;
    let buy_ixs = StonkFunInstructionBuilder.build_buy_instructions(&buy).await.unwrap();
    let meme_min_out = {
        let swap = buy_ixs
            .iter()
            .rev()
            .find(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM)
            .unwrap();
        u64::from_le_bytes(swap.data[16..24].try_into().unwrap())
    };
    let sell_amount = (meme_min_out / 2).max(1);

    let mut sell = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Sell,
        fixtures::GRAD_MEME_KNOTS,
        fixtures::GRAD_QUOTE_STONK,
        sell_amount,
        300,
        DexParamEnum::StonkFunSwap(graduated),
    );
    sell.create_input_mint_ata = false;
    sell.close_input_mint_ata = false;
    sell.create_output_mint_ata = false;
    sell.close_output_mint_ata = false;
    let sell_ixs = StonkFunInstructionBuilder.build_sell_instructions(&sell).await.unwrap();

    let mut business = hop1_ixs;
    business.extend(buy_ixs);
    business.extend(sell_ixs);
    let alt = mainnet_sim::load_alt(&rpc, &fixtures::WSOL_STONK_LUT).await;
    mainnet_sim::run_business_sim(
        &rpc,
        &wallet,
        business,
        &[alt],
        "stonkfun swap graduated buy+sell",
    )
    .await;
}

#[tokio::test]
async fn bonk_and_launchlab_aliases_mainnet_simulates_curve_buy_after_sol_hop() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let curve = { let Some(v) = mainnet_sim::load_stonkfun_curve(&rpc, &fixtures::CURVE_POOL).await else { return; }; v };
    let sol_hop =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::WSOL_CARDS_CPMM).await else { return; }; v };

    let hop1 = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::CURVE_QUOTE_CARDS,
        20_000,
        500,
        DexParamEnum::RaydiumCpmm(sol_hop),
    );
    let hop1_ixs =
        crate::instruction::raydium_cpmm::RaydiumCpmmInstructionBuilder
            .build_buy_instructions(&hop1)
            .await
            .unwrap();
    let cards_min_out = {
        let swap = hop1_ixs
            .iter()
            .find(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM)
            .unwrap();
        u64::from_le_bytes(swap.data[16..24].try_into().unwrap())
    };

    // Same live pool through Bonk and LaunchLab aliases (shared LaunchLab program).
    for (label, protocol) in [
        ("Bonk", DexParamEnum::Bonk(curve.clone())),
        ("LaunchLab", DexParamEnum::LaunchLab(curve.clone())),
    ] {
        let mut hop2 = mainnet_sim::swap_params(
            wallet.clone(),
            TradeType::Buy,
            fixtures::CURVE_QUOTE_CARDS,
            fixtures::CURVE_MEME,
            cards_min_out,
            500,
            protocol,
        );
        hop2.create_input_mint_ata = false;
        let hop2_ixs = crate::instruction::bonk::BonkInstructionBuilder
            .build_buy_instructions(&hop2)
            .await
            .unwrap_or_else(|e| panic!("{label} buy build failed: {e}"));
        assert!(
            hop2_ixs
                .iter()
                .any(|ix| ix.program_id == crate::instruction::utils::bonk::accounts::BONK),
            "{label} must emit LaunchLab program ix"
        );

        let mut business = hop1_ixs.clone();
        business.extend(hop2_ixs);
        let alt = mainnet_sim::load_alt(&rpc, &fixtures::WSOL_CARDS_LUT).await;
        mainnet_sim::run_business_sim(
            &rpc,
            &wallet,
            business,
            &[alt],
            &format!("{label} alias curve buy"),
        )
        .await;
    }
}
