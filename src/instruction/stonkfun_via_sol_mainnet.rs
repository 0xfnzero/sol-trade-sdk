//! Mainnet simulation tests for StonkFun `SOL ↔ quote ↔ meme` (ViaSol) routing.

#![cfg(test)]

use crate::{
    common::mainnet_sim::{self, fixtures},
    instruction::{
        stonkfun::StonkFunInstructionBuilder,
        utils::raydium_cpmm::{accounts as cpmm_accounts, SWAP_BASE_IN_DISCRIMINATOR},
    },
    swqos::TradeType,
    trading::core::{
        params::{
            DexParamEnum, StonkFunViaSolParams,
        },
        traits::InstructionBuilder,
    },
    BuyAmount, SellAmount, SimpleBuyParams, SimpleSellParams, TradeTokenType,
};
use solana_sdk::hash::Hash;

#[tokio::test]
async fn via_sol_graduated_mainnet_creates_wallet_and_simulates_buy() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let graduated =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::GRAD_POOL).await else { return; }; v };
    let sol_hop =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::WSOL_STONK_CPMM).await else { return; }; v };
    assert!(sol_hop.base_reserve > 0 && sol_hop.quote_reserve > 0);

    let via = StonkFunViaSolParams::graduated_with_cpmm(graduated, sol_hop);
    let params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::GRAD_MEME_KNOTS,
        50_000,
        300,
        DexParamEnum::StonkFunViaSol(via),
    );
    let business = StonkFunInstructionBuilder.build_buy_instructions(&params).await.unwrap();

    let cpmm_ixs: Vec<_> =
        business.iter().filter(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM).collect();
    assert_eq!(cpmm_ixs.len(), 2);
    assert_eq!(&cpmm_ixs[0].data[..8], SWAP_BASE_IN_DISCRIMINATOR);
    let hop1_min = u64::from_le_bytes(cpmm_ixs[0].data[16..24].try_into().unwrap());
    let hop2_in = u64::from_le_bytes(cpmm_ixs[1].data[8..16].try_into().unwrap());
    assert_eq!(hop1_min, hop2_in);
    assert!(hop1_min > 0);

    let alt = mainnet_sim::load_alt(&rpc, &fixtures::WSOL_STONK_LUT).await;
    mainnet_sim::run_business_sim(&rpc, &wallet, business, &[alt], "via_sol graduated buy").await;
}

#[tokio::test]
async fn via_sol_curve_mainnet_creates_wallet_and_simulates_buy() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let curve = { let Some(v) = mainnet_sim::load_stonkfun_curve(&rpc, &fixtures::CURVE_POOL).await else { return; }; v };
    let sol_hop =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::WSOL_CARDS_CPMM).await else { return; }; v };
    assert_eq!(curve.quote_mint, fixtures::CURVE_QUOTE_CARDS);

    let via = StonkFunViaSolParams::curve_with_cpmm(curve, sol_hop);
    let params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::CURVE_MEME,
        10_000,
        500,
        DexParamEnum::StonkFunViaSol(via),
    );
    let business = StonkFunInstructionBuilder.build_buy_instructions(&params).await.unwrap();

    assert!(business.iter().any(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM));
    assert!(business
        .iter()
        .any(|ix| ix.program_id == crate::instruction::utils::bonk::accounts::BONK));

    let alt = mainnet_sim::load_alt(&rpc, &fixtures::WSOL_CARDS_LUT).await;
    mainnet_sim::run_business_sim(&rpc, &wallet, business, &[alt], "via_sol curve buy").await;
}

#[tokio::test]
async fn via_sol_graduated_mainnet_creates_wallet_and_simulates_sell_after_virtual_buy() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let graduated =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::GRAD_POOL).await else { return; }; v };
    let sol_hop =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::WSOL_STONK_CPMM).await else { return; }; v };
    let via_buy = StonkFunViaSolParams::graduated_with_cpmm(graduated.clone(), sol_hop.clone());
    let via_sell = StonkFunViaSolParams::graduated_with_cpmm(graduated, sol_hop);

    let buy_params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::GRAD_MEME_KNOTS,
        50_000,
        300,
        DexParamEnum::StonkFunViaSol(via_buy),
    );
    let buy_ixs = StonkFunInstructionBuilder.build_buy_instructions(&buy_params).await.unwrap();
    let buy_cpmm: Vec<_> =
        buy_ixs.iter().filter(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM).collect();
    let meme_min_out = u64::from_le_bytes(buy_cpmm[1].data[16..24].try_into().unwrap());
    let sell_amount = (meme_min_out / 2).max(1);

    let sell_params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Sell,
        fixtures::GRAD_MEME_KNOTS,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        sell_amount,
        300,
        DexParamEnum::StonkFunViaSol(via_sell),
    );
    let mut sell = sell_params;
    sell.create_input_mint_ata = false;
    sell.close_input_mint_ata = false;
    sell.create_output_mint_ata = false;
    sell.close_output_mint_ata = false;
    let sell_ixs = StonkFunInstructionBuilder.build_sell_instructions(&sell).await.unwrap();

    let mut business = buy_ixs;
    business.extend(sell_ixs);
    let alt = mainnet_sim::load_alt(&rpc, &fixtures::WSOL_STONK_LUT).await;
    mainnet_sim::run_business_sim(&rpc, &wallet, business, &[alt], "via_sol graduated buy+sell")
        .await;
}

#[tokio::test]
async fn via_sol_graduated_mainnet_hot_path_minimal_simulates_buy() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let graduated =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::GRAD_POOL).await else { return; }; v };
    let sol_hop =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::WSOL_STONK_CPMM).await else { return; }; v };
    let via = StonkFunViaSolParams::graduated_with_cpmm(graduated, sol_hop);

    let setup = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::GRAD_MEME_KNOTS,
        50_000,
        300,
        DexParamEnum::StonkFunViaSol(via.clone()),
    );
    let setup_ixs = StonkFunInstructionBuilder.build_buy_instructions(&setup).await.unwrap();
    let mut ata_and_wrap: Vec<_> = setup_ixs
        .into_iter()
        .filter(|ix| ix.program_id != cpmm_accounts::RAYDIUM_CPMM)
        .collect();

    let mut hot = setup;
    hot.create_input_mint_ata = false;
    hot.close_input_mint_ata = false;
    hot.create_output_mint_ata = false;
    hot.close_output_mint_ata = false;
    let swap_ixs = StonkFunInstructionBuilder.build_buy_instructions(&hot).await.unwrap();
    assert!(
        swap_ixs.iter().all(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM),
        "HotPathMinimal ViaSol buy should be swap-only, got {:?}",
        swap_ixs.iter().map(|ix| ix.program_id).collect::<Vec<_>>()
    );
    assert_eq!(swap_ixs.len(), 2);

    ata_and_wrap.extend(swap_ixs);
    let alt = mainnet_sim::load_alt(&rpc, &fixtures::WSOL_STONK_LUT).await;
    mainnet_sim::run_business_sim(
        &rpc,
        &wallet,
        ata_and_wrap,
        &[alt],
        "via_sol graduated HotPathMinimal buy",
    )
    .await;
}

#[tokio::test]
async fn via_sol_graduated_mainnet_higher_slippage_and_larger_input_simulates() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let graduated =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::GRAD_POOL).await else { return; }; v };
    let sol_hop =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::WSOL_STONK_CPMM).await else { return; }; v };
    let via = StonkFunViaSolParams::graduated_with_cpmm(graduated, sol_hop);
    let params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::GRAD_MEME_KNOTS,
        200_000,
        800,
        DexParamEnum::StonkFunViaSol(via),
    );
    let business = StonkFunInstructionBuilder.build_buy_instructions(&params).await.unwrap();
    let alt = mainnet_sim::load_alt(&rpc, &fixtures::WSOL_STONK_LUT).await;
    mainnet_sim::run_business_sim(
        &rpc,
        &wallet,
        business,
        &[alt],
        "via_sol graduated buy wider slip",
    )
    .await;
}

#[tokio::test]
async fn via_sol_curve_mainnet_creates_wallet_and_simulates_sell_after_virtual_buy() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let curve = { let Some(v) = mainnet_sim::load_stonkfun_curve(&rpc, &fixtures::CURVE_POOL).await else { return; }; v };
    let sol_hop =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::WSOL_CARDS_CPMM).await else { return; }; v };
    let via_buy = StonkFunViaSolParams::curve_with_cpmm(curve.clone(), sol_hop.clone());
    let via_sell = StonkFunViaSolParams::curve_with_cpmm(curve.clone(), sol_hop);

    let buy_params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::CURVE_MEME,
        10_000,
        500,
        DexParamEnum::StonkFunViaSol(via_buy),
    );
    let buy_ixs = StonkFunInstructionBuilder.build_buy_instructions(&buy_params).await.unwrap();

    // LaunchLab buy exact-in encodes min_out at bytes [16..24].
    let curve_ix = buy_ixs
        .iter()
        .find(|ix| ix.program_id == crate::instruction::utils::bonk::accounts::BONK)
        .expect("curve buy ix");
    let meme_min_out = u64::from_le_bytes(curve_ix.data[16..24].try_into().unwrap());
    let sell_amount = (meme_min_out / 2).max(1);

    // Full ViaSol sell is 2 hops (meme→quote→SOL). Concatenating with the buy
    // overflows the simulateTransaction size cap for LaunchLab-heavy txs, so:
    // 1) assert the ViaSol sell composition (2 program legs)
    // 2) simulate buy + curve-only sell (meme→quote) which still exercises the
    //    LaunchLab sell path against live pool state.
    let mut sell_via = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Sell,
        fixtures::CURVE_MEME,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        sell_amount,
        500,
        DexParamEnum::StonkFunViaSol(via_sell),
    );
    sell_via.create_input_mint_ata = false;
    sell_via.close_input_mint_ata = false;
    sell_via.create_output_mint_ata = false;
    sell_via.close_output_mint_ata = false;
    let sell_via_ixs =
        StonkFunInstructionBuilder.build_sell_instructions(&sell_via).await.unwrap();
    assert!(
        sell_via_ixs.iter().any(|ix| ix.program_id == crate::instruction::utils::bonk::accounts::BONK),
        "via_sol curve sell must include LaunchLab leg"
    );
    assert!(
        sell_via_ixs.iter().any(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM),
        "via_sol curve sell must include SOL hop CPMM leg"
    );

    let mut curve_only = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Sell,
        fixtures::CURVE_MEME,
        fixtures::CURVE_QUOTE_CARDS,
        sell_amount,
        500,
        DexParamEnum::StonkFun(curve),
    );
    curve_only.create_input_mint_ata = false;
    curve_only.close_input_mint_ata = false;
    curve_only.create_output_mint_ata = false;
    curve_only.close_output_mint_ata = false;
    let sell_ixs =
        StonkFunInstructionBuilder.build_sell_instructions(&curve_only).await.unwrap();

    let mut business = buy_ixs;
    business.extend(sell_ixs);
    let alt = mainnet_sim::load_alt(&rpc, &fixtures::WSOL_CARDS_LUT).await;
    mainnet_sim::run_business_sim(
        &rpc,
        &wallet,
        business,
        &[alt],
        "via_sol curve buy + curve sell",
    )
    .await;
}

#[tokio::test]
async fn via_sol_simple_helpers_still_wire_dex_type() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let graduated =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::GRAD_POOL).await else { return; }; v };
    let sol_hop =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::WSOL_STONK_CPMM).await else { return; }; v };
    let via = StonkFunViaSolParams::graduated_with_cpmm(graduated, sol_hop);
    let buy = SimpleBuyParams::stonkfun_with_sol(
        fixtures::GRAD_MEME_KNOTS,
        BuyAmount::ExactInput(50_000),
        via.clone(),
        Hash::new_unique(),
        crate::common::GasFeeStrategy::new(),
    );
    assert!(matches!(buy.pay_with, TradeTokenType::SOL));
    assert!(matches!(buy.extension_params, DexParamEnum::StonkFunViaSol(_)));

    let sell = SimpleSellParams::stonkfun_to_sol(
        fixtures::GRAD_MEME_KNOTS,
        SellAmount::ExactInput(1_000),
        via,
        Hash::new_unique(),
        crate::common::GasFeeStrategy::new(),
    );
    assert!(matches!(sell.receive_as, TradeTokenType::SOL));
}

#[tokio::test]
async fn via_sol_graduated_mainnet_sell_closes_wsol() {
    if !mainnet_sim::enabled() {
        return;
    }

    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let graduated =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::GRAD_POOL).await else { return; }; v };
    let sol_hop =
        { let Some(v) = mainnet_sim::load_cpmm(&rpc, &fixtures::WSOL_STONK_CPMM).await else { return; }; v };
    let via_buy = StonkFunViaSolParams::graduated_with_cpmm(graduated.clone(), sol_hop.clone());
    let via_sell = StonkFunViaSolParams::graduated_with_cpmm(graduated, sol_hop);

    let buy = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::GRAD_MEME_KNOTS,
        50_000,
        300,
        DexParamEnum::StonkFunViaSol(via_buy),
    );
    let buy_ixs = StonkFunInstructionBuilder.build_buy_instructions(&buy).await.unwrap();
    let meme_min_out = {
        let swap = buy_ixs
            .iter()
            .rev()
            .find(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM)
            .unwrap();
        u64::from_le_bytes(swap.data[16..24].try_into().unwrap())
    };

    let mut sell = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Sell,
        fixtures::GRAD_MEME_KNOTS,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        (meme_min_out / 2).max(1),
        300,
        DexParamEnum::StonkFunViaSol(via_sell),
    );
    sell.create_input_mint_ata = false;
    sell.close_input_mint_ata = false;
    sell.create_output_mint_ata = false;
    // Auto-style sell: unwrap WSOL back to native SOL after the SOL hop.
    sell.close_output_mint_ata = true;
    let sell_ixs = StonkFunInstructionBuilder.build_sell_instructions(&sell).await.unwrap();
    assert!(
        sell_ixs.iter().any(|ix| {
            ix.program_id == crate::constants::TOKEN_PROGRAM
                && ix.data.first().copied() == Some(9) // CloseAccount
        }),
        "ViaSol sell with close_output should unwrap WSOL"
    );

    let mut business = buy_ixs;
    business.extend(sell_ixs);
    let alt = mainnet_sim::load_alt(&rpc, &fixtures::WSOL_STONK_LUT).await;
    mainnet_sim::run_business_sim(
        &rpc,
        &wallet,
        business,
        &[alt],
        "via_sol graduated buy+sell close WSOL",
    )
    .await;
}
