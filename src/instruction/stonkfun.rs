//! StonkFun trading across its LaunchLab curve and graduated CPMM pools.
//!
//! Also supports atomic SOL ↔ quote ↔ meme routing via
//! [`DexParamEnum::StonkFunViaSol`] so wallets do not need to pre-hold stock
//! quote tokens.

use super::{
    bonk::BonkInstructionBuilder, meteora_damm_v2::MeteoraDammV2InstructionBuilder,
    meteora_dbc::MeteoraDbcInstructionBuilder, meteora_dlmm::MeteoraDlmmInstructionBuilder,
    raydium_amm_v4::RaydiumAmmV4InstructionBuilder, raydium_clmm::RaydiumClmmInstructionBuilder,
    raydium_cpmm::RaydiumCpmmInstructionBuilder, whirlpool::WhirlpoolInstructionBuilder,
};
use crate::{
    constants::trade::trade::DEFAULT_SLIPPAGE,
    trading::core::{
        params::{
            DexParamEnum, HopSpot, MeteoraDammV2Params, MeteoraDbcParams, RaydiumAmmV4Params,
            RaydiumCpmmParams, StonkFunMemeLeg, StonkFunSolHop, StonkFunViaSolParams, SwapParams,
        },
        traits::InstructionBuilder,
    },
    utils::calc::{
        bonk::{get_buy_quote, get_sell_min_amount_out},
        common::calculate_min_amount_out,
        meteora_damm_v2::quote_exact_in as quote_damm_v2_exact_in,
        raydium_amm_v4::compute_swap_amount_for_pool as compute_amm_v4_swap_amount_for_pool,
        raydium_cpmm::compute_swap_amount_for_pool,
    },
};
use anyhow::{anyhow, Result};
use solana_sdk::{instruction::Instruction, pubkey::Pubkey, signer::Signer};

/// User-facing StonkFun builder that selects the curve, graduated swap, or
/// SOL-routed two-hop path from the protocol params variant.
pub struct StonkFunInstructionBuilder;

fn normalize_native_sol(mint: Pubkey) -> Pubkey {
    if mint == crate::constants::SOL_TOKEN_ACCOUNT {
        crate::constants::WSOL_TOKEN_ACCOUNT
    } else {
        mint
    }
}

fn is_native_sol(mint: Pubkey) -> bool {
    mint == crate::constants::SOL_TOKEN_ACCOUNT || mint == crate::constants::WSOL_TOKEN_ACCOUNT
}

fn curve_quote_mint(params: &crate::trading::core::params::BonkParams) -> Result<Pubkey> {
    if params.quote_mint != Pubkey::default() {
        return Ok(normalize_native_sol(params.quote_mint));
    }
    if params.global_config
        == crate::instruction::utils::bonk::accounts::USD1_GLOBAL_CONFIG
    {
        return Ok(crate::constants::USD1_TOKEN_ACCOUNT);
    }
    Ok(crate::constants::WSOL_TOKEN_ACCOUNT)
}

fn graduated_quote_mint(pool: &RaydiumCpmmParams, meme_mint: Pubkey) -> Result<Pubkey> {
    let meme = normalize_native_sol(meme_mint);
    if pool.base_mint == meme {
        Ok(normalize_native_sol(pool.quote_mint))
    } else if pool.quote_mint == meme {
        Ok(normalize_native_sol(pool.base_mint))
    } else {
        Err(anyhow!(
            "Meme mint {} is not part of graduated StonkFun pool {}/{}",
            meme,
            pool.base_mint,
            pool.quote_mint
        ))
    }
}

/// The side of a DAMM v2 pool that is not `meme_mint`.
fn damm_v2_quote_mint(pool: &MeteoraDammV2Params, meme_mint: Pubkey) -> Result<Pubkey> {
    let meme = normalize_native_sol(meme_mint);
    if pool.token_a_mint == meme {
        Ok(normalize_native_sol(pool.token_b_mint))
    } else if pool.token_b_mint == meme {
        Ok(normalize_native_sol(pool.token_a_mint))
    } else {
        Err(anyhow!(
            "Meme mint {} is not part of Meteora DAMM v2 pool {}/{}",
            meme,
            pool.token_a_mint,
            pool.token_b_mint
        ))
    }
}

fn meme_leg_quote_mint(meme_leg: &StonkFunMemeLeg, meme_mint: Pubkey) -> Result<Pubkey> {
    match meme_leg {
        StonkFunMemeLeg::Curve(params) => curve_quote_mint(params),
        StonkFunMemeLeg::Graduated(params) => graduated_quote_mint(params, meme_mint),
        StonkFunMemeLeg::MeteoraDbc(params) => Ok(normalize_native_sol(params.quote_mint)),
        StonkFunMemeLeg::MeteoraDammV2(params) => damm_v2_quote_mint(params, meme_mint),
    }
}

fn meme_leg_as_dex_param(meme_leg: &StonkFunMemeLeg) -> DexParamEnum {
    match meme_leg {
        StonkFunMemeLeg::Curve(params) => DexParamEnum::StonkFun(params.clone()),
        StonkFunMemeLeg::Graduated(params) => DexParamEnum::StonkFunSwap(params.clone()),
        StonkFunMemeLeg::MeteoraDbc(params) => DexParamEnum::MeteoraDbc(params.clone()),
        StonkFunMemeLeg::MeteoraDammV2(params) => DexParamEnum::MeteoraDammV2(params.clone()),
    }
}

/// Minimum output of a Meteora DBC leg, quoted on the curve its params carry.
fn dbc_leg_min_out(
    pool: &MeteoraDbcParams,
    is_buy: bool,
    amount_in: u64,
    slippage_basis_points: u64,
) -> Result<u64> {
    let curve = pool
        .quote
        .as_ref()
        .ok_or_else(|| anyhow!("Meteora DBC pool {} needs its curve to quote a leg", pool.pool))?;
    let quote = curve.quote_exact_in(is_buy, amount_in)?;
    Ok(calculate_min_amount_out(quote.amount_out, slippage_basis_points))
}

/// Minimum output of a Meteora DAMM v2 leg, quoted on the state its params carry.
fn damm_v2_leg_min_out(
    pool: &MeteoraDammV2Params,
    input_mint: Pubkey,
    amount_in: u64,
    slippage_basis_points: u64,
) -> Result<u64> {
    let state = pool.quote.as_ref().ok_or_else(|| {
        anyhow!("Meteora DAMM v2 pool {} needs its state to quote a leg", pool.pool)
    })?;
    let a_to_b = normalize_native_sol(input_mint) == pool.token_a_mint;
    let quote = quote_damm_v2_exact_in(state, a_to_b, amount_in)?;
    Ok(calculate_min_amount_out(quote.amount_out, slippage_basis_points))
}

fn sol_hop_as_dex_param(sol_hop: &StonkFunSolHop) -> DexParamEnum {
    match sol_hop {
        StonkFunSolHop::RaydiumCpmm(params) => DexParamEnum::RaydiumCpmm(params.clone()),
        StonkFunSolHop::RaydiumAmmV4(params) => DexParamEnum::RaydiumAmmV4(params.clone()),
        StonkFunSolHop::RaydiumClmm(params) => DexParamEnum::RaydiumClmm(params.clone()),
        StonkFunSolHop::OrcaWhirlpool(params) => DexParamEnum::OrcaWhirlpool(params.clone()),
        StonkFunSolHop::MeteoraDlmm(params) => DexParamEnum::MeteoraDlmm(params.clone()),
    }
}

/// Concentrated-liquidity builders take their minimum output from the
/// caller; CPMM and AMM v4 price their own from reserves.
fn hop_fixed_output(sol_hop: &StonkFunSolHop, min_out: u64) -> Option<u64> {
    match sol_hop {
        StonkFunSolHop::RaydiumCpmm(_) | StonkFunSolHop::RaydiumAmmV4(_) => None,
        StonkFunSolHop::RaydiumClmm(_)
        | StonkFunSolHop::OrcaWhirlpool(_)
        | StonkFunSolHop::MeteoraDlmm(_) => Some(min_out),
    }
}

fn cpmm_is_base_in(pool: &RaydiumCpmmParams, input_mint: Pubkey, output_mint: Pubkey) -> Result<bool> {
    let input = normalize_native_sol(input_mint);
    let output = normalize_native_sol(output_mint);
    if input == pool.base_mint && output == pool.quote_mint {
        Ok(true)
    } else if input == pool.quote_mint && output == pool.base_mint {
        Ok(false)
    } else {
        Err(anyhow!(
            "Requested swap pair {}/{} does not match Raydium CPMM pool {}/{}",
            input,
            output,
            pool.base_mint,
            pool.quote_mint
        ))
    }
}

fn amm_v4_is_coin_in(
    pool: &RaydiumAmmV4Params,
    input_mint: Pubkey,
    output_mint: Pubkey,
) -> Result<bool> {
    let input = normalize_native_sol(input_mint);
    let output = normalize_native_sol(output_mint);
    if input == pool.coin_mint && output == pool.pc_mint {
        Ok(true)
    } else if input == pool.pc_mint && output == pool.coin_mint {
        Ok(false)
    } else {
        Err(anyhow!(
            "Requested swap pair {}/{} does not match Raydium AMM v4 pool {}/{}",
            input,
            output,
            pool.coin_mint,
            pool.pc_mint
        ))
    }
}

fn hop_mints(sol_hop: &StonkFunSolHop) -> (Pubkey, Pubkey) {
    let (first, second) = sol_hop.mints();
    (normalize_native_sol(first), normalize_native_sol(second))
}

/// Mints along the route from WSOL to `quote_mint`: WSOL, the currency of a
/// second hop if there is one, then the quote.
fn sol_route_mints(via: &StonkFunViaSolParams, quote_mint: Pubkey) -> Result<Vec<Pubkey>> {
    let wsol = crate::constants::WSOL_TOKEN_ACCOUNT;
    let quote = normalize_native_sol(quote_mint);
    let (a, b) = hop_mints(&via.sol_hop);
    let first = match (a == wsol, b == wsol) {
        (true, false) => b,
        (false, true) => a,
        _ => {
            return Err(anyhow!(
                "SOL hop pool {}/{} does not match WSOL/{}",
                a,
                b,
                quote
            ))
        }
    };
    match &via.quote_hop {
        None if first == quote => Ok(vec![wsol, quote]),
        None => Err(anyhow!(
            "SOL hop pool {}/{} does not match WSOL/{}",
            a,
            b,
            quote
        )),
        Some(quote_hop) => {
            let (c, d) = hop_mints(quote_hop);
            if first != quote && ((c == first && d == quote) || (c == quote && d == first)) {
                Ok(vec![wsol, first, quote])
            } else {
                Err(anyhow!(
                    "Quote hop pool {}/{} does not match {}/{}",
                    c,
                    d,
                    first,
                    quote
                ))
            }
        }
    }
}

/// The route's pools, from the SOL side.
fn sol_route_hops(via: &StonkFunViaSolParams) -> Vec<&StonkFunSolHop> {
    std::iter::once(&via.sol_hop).chain(via.quote_hop.as_ref()).collect()
}

/// Minimum output of a swap quoted at a concentrated-liquidity pool's spot price.
fn spot_min_out(
    sol_hop: &StonkFunSolHop,
    spot: Option<HopSpot>,
    amount_in: u64,
    input_mint: Pubkey,
    output_mint: Pubkey,
    slippage_basis_points: u64,
) -> Result<u64> {
    let spot = spot.ok_or_else(|| {
        anyhow!("Hop pool {} needs its spot price to quote the hop", sol_hop.pool())
    })?;
    let (first, second) = hop_mints(sol_hop);
    let (input, output) = (normalize_native_sol(input_mint), normalize_native_sol(output_mint));
    let first_to_second = if input == first && output == second {
        true
    } else if input == second && output == first {
        false
    } else {
        return Err(anyhow!(
            "Requested swap pair {}/{} does not match hop pool {}/{}",
            input,
            output,
            first,
            second
        ));
    };
    Ok(spot.min_amount_out(amount_in, first_to_second, slippage_basis_points))
}

fn sol_hop_min_out(
    sol_hop: &StonkFunSolHop,
    amount_in: u64,
    input_mint: Pubkey,
    output_mint: Pubkey,
    slippage_basis_points: u64,
) -> Result<u64> {
    match sol_hop {
        StonkFunSolHop::RaydiumCpmm(pool) => {
            let is_base_in = cpmm_is_base_in(pool, input_mint, output_mint)?;
            Ok(compute_swap_amount_for_pool(pool, is_base_in, amount_in, slippage_basis_points)?
                .min_amount_out)
        }
        StonkFunSolHop::RaydiumAmmV4(pool) => {
            let is_coin_in = amm_v4_is_coin_in(pool, input_mint, output_mint)?;
            Ok(compute_amm_v4_swap_amount_for_pool(
                pool,
                is_coin_in,
                amount_in,
                slippage_basis_points,
            )?
            .min_amount_out)
        }
        StonkFunSolHop::RaydiumClmm(pool) => {
            let quote = pool.quote_exact_in(&normalize_native_sol(input_mint), amount_in)?;
            Ok(calculate_min_amount_out(quote.amount_out, slippage_basis_points))
        }
        StonkFunSolHop::OrcaWhirlpool(pool) => spot_min_out(
            sol_hop,
            pool.spot,
            amount_in,
            input_mint,
            output_mint,
            slippage_basis_points,
        ),
        StonkFunSolHop::MeteoraDlmm(pool) if pool.quote_state.is_some() => {
            let quote = pool.quote_exact_in(&normalize_native_sol(input_mint), amount_in)?;
            Ok(calculate_min_amount_out(quote.amount_out, slippage_basis_points))
        }
        StonkFunSolHop::MeteoraDlmm(pool) => spot_min_out(
            sol_hop,
            pool.spot,
            amount_in,
            input_mint,
            output_mint,
            slippage_basis_points,
        ),
    }
}

/// The hop as built for `amount_in`, and its minimum output: a CLMM hop gets
/// the tick arrays its exact quote crosses, a DLMM hop quoted exactly the bin
/// arrays its quote walks through.
fn quote_hop(
    sol_hop: &StonkFunSolHop,
    amount_in: u64,
    input_mint: Pubkey,
    output_mint: Pubkey,
    slippage_basis_points: u64,
) -> Result<(StonkFunSolHop, u64)> {
    match sol_hop {
        StonkFunSolHop::RaydiumClmm(pool) => {
            let quote = pool.quote_exact_in(&normalize_native_sol(input_mint), amount_in)?;
            let mut pool = pool.clone();
            pool.tick_arrays = quote.tick_arrays;
            Ok((
                StonkFunSolHop::RaydiumClmm(pool),
                calculate_min_amount_out(quote.amount_out, slippage_basis_points),
            ))
        }
        StonkFunSolHop::MeteoraDlmm(pool) if pool.quote_state.is_some() => {
            let quote = pool.quote_exact_in(&normalize_native_sol(input_mint), amount_in)?;
            let mut pool = pool.clone();
            pool.bin_arrays = quote.bin_arrays;
            Ok((
                StonkFunSolHop::MeteoraDlmm(pool),
                calculate_min_amount_out(quote.amount_out, slippage_basis_points),
            ))
        }
        _ => Ok((
            sol_hop.clone(),
            sol_hop_min_out(sol_hop, amount_in, input_mint, output_mint, slippage_basis_points)?,
        )),
    }
}

fn meme_leg_buy_min_out(
    meme_leg: &StonkFunMemeLeg,
    quote_amount_in: u64,
    meme_mint: Pubkey,
    slippage_basis_points: u64,
) -> Result<u64> {
    match meme_leg {
        StonkFunMemeLeg::Curve(params) => Ok(get_buy_quote(
            quote_amount_in,
            params,
            0,
            slippage_basis_points as u128,
        )?
        .minimum_amount_out),
        StonkFunMemeLeg::Graduated(pool) => {
            let quote_mint = graduated_quote_mint(pool, meme_mint)?;
            let is_base_in = cpmm_is_base_in(pool, quote_mint, meme_mint)?;
            Ok(compute_swap_amount_for_pool(
                pool,
                is_base_in,
                quote_amount_in,
                slippage_basis_points,
            )?
            .min_amount_out)
        }
        StonkFunMemeLeg::MeteoraDbc(pool) => {
            dbc_leg_min_out(pool, true, quote_amount_in, slippage_basis_points)
        }
        StonkFunMemeLeg::MeteoraDammV2(pool) => {
            let quote_mint = damm_v2_quote_mint(pool, meme_mint)?;
            damm_v2_leg_min_out(pool, quote_mint, quote_amount_in, slippage_basis_points)
        }
    }
}

fn meme_leg_sell_min_out(
    meme_leg: &StonkFunMemeLeg,
    meme_amount_in: u64,
    meme_mint: Pubkey,
    slippage_basis_points: u64,
) -> Result<u64> {
    match meme_leg {
        StonkFunMemeLeg::Curve(params) => {
            get_sell_min_amount_out(meme_amount_in, params, 0, slippage_basis_points as u128)
        }
        StonkFunMemeLeg::Graduated(pool) => {
            let quote_mint = graduated_quote_mint(pool, meme_mint)?;
            let is_base_in = cpmm_is_base_in(pool, meme_mint, quote_mint)?;
            Ok(compute_swap_amount_for_pool(
                pool,
                is_base_in,
                meme_amount_in,
                slippage_basis_points,
            )?
            .min_amount_out)
        }
        StonkFunMemeLeg::MeteoraDbc(pool) => {
            dbc_leg_min_out(pool, false, meme_amount_in, slippage_basis_points)
        }
        StonkFunMemeLeg::MeteoraDammV2(pool) => {
            damm_v2_leg_min_out(pool, meme_mint, meme_amount_in, slippage_basis_points)
        }
    }
}

/// The trade's params with the hop's slippage, for the SOL↔quote leg.
fn hop_swap_params(
    params: &SwapParams,
    via: &StonkFunViaSolParams,
    slippage: u64,
) -> (SwapParams, u64) {
    let hop_slippage = via.hop_slippage_basis_points.unwrap_or(slippage);
    let mut hop = params.clone();
    hop.slippage_basis_points = Some(hop_slippage);
    (hop, hop_slippage)
}

async fn build_leg_buy(
    params: &SwapParams,
    protocol_params: DexParamEnum,
    input_mint: Pubkey,
    output_mint: Pubkey,
    input_amount: u64,
    create_input_mint_ata: bool,
    close_input_mint_ata: bool,
    create_output_mint_ata: bool,
    close_output_mint_ata: bool,
    fixed_output_amount: Option<u64>,
) -> Result<Vec<Instruction>> {
    let mut leg = params.clone();
    leg.protocol_params = protocol_params;
    leg.input_mint = input_mint;
    leg.output_mint = output_mint;
    leg.input_amount = Some(input_amount);
    leg.create_input_mint_ata = create_input_mint_ata;
    leg.close_input_mint_ata = close_input_mint_ata;
    leg.create_output_mint_ata = create_output_mint_ata;
    leg.close_output_mint_ata = close_output_mint_ata;
    leg.fixed_output_amount = fixed_output_amount;
    StonkFunInstructionBuilder.build_buy_instructions(&leg).await
}

async fn build_leg_sell(
    params: &SwapParams,
    protocol_params: DexParamEnum,
    input_mint: Pubkey,
    output_mint: Pubkey,
    input_amount: u64,
    create_input_mint_ata: bool,
    close_input_mint_ata: bool,
    create_output_mint_ata: bool,
    close_output_mint_ata: bool,
    fixed_output_amount: Option<u64>,
) -> Result<Vec<Instruction>> {
    let mut leg = params.clone();
    leg.protocol_params = protocol_params;
    leg.input_mint = input_mint;
    leg.output_mint = output_mint;
    leg.input_amount = Some(input_amount);
    leg.create_input_mint_ata = create_input_mint_ata;
    leg.close_input_mint_ata = close_input_mint_ata;
    leg.create_output_mint_ata = create_output_mint_ata;
    leg.close_output_mint_ata = close_output_mint_ata;
    leg.fixed_output_amount = fixed_output_amount;
    StonkFunInstructionBuilder.build_sell_instructions(&leg).await
}

async fn build_buy_via_sol(
    params: &SwapParams,
    via: &StonkFunViaSolParams,
) -> Result<Vec<Instruction>> {
    if !is_native_sol(params.input_mint) {
        return Err(anyhow!(
            "StonkFunViaSol buy expects SOL/WSOL input mint, got {}",
            params.input_mint
        ));
    }
    let meme_mint = params.output_mint;
    let quote_mint = meme_leg_quote_mint(&via.meme_leg, meme_mint)?;
    let sol_amount = params
        .input_amount
        .filter(|&a| a > 0)
        .ok_or_else(|| anyhow!("StonkFunViaSol buy requires a non-zero SOL input amount"))?;
    let slippage = params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE);
    let wsol = crate::constants::WSOL_TOKEN_ACCOUNT;

    // Quote is already SOL: collapse to a single meme-leg buy.
    if is_native_sol(quote_mint) {
        return build_leg_buy(
            params,
            meme_leg_as_dex_param(&via.meme_leg),
            wsol,
            meme_mint,
            sol_amount,
            params.create_input_mint_ata,
            params.close_input_mint_ata,
            params.create_output_mint_ata,
            false,
            params.fixed_output_amount,
        )
        .await;
    }

    let route = sol_route_mints(via, quote_mint)?;
    let hops = sol_route_hops(via);
    let (hop_params, hop_slippage) = hop_swap_params(params, via, slippage);

    // Each leg spends the minimum output of the one before, so none can
    // overspend the account it draws from.
    let mut bridges = Vec::with_capacity(hops.len());
    let mut amount = sol_amount;
    for (sol_hop, pair) in hops.iter().zip(route.windows(2)) {
        let (hop, min_out) = quote_hop(sol_hop, amount, pair[0], pair[1], hop_slippage)?;
        amount = min_out;
        if amount == 0 {
            return Err(anyhow!("StonkFunViaSol SOL hop produced zero quote output"));
        }
        bridges.push((hop, amount));
    }
    let quote_bridge = amount;

    // Validate the meme leg can absorb that quote amount (also warms error paths).
    let _ = meme_leg_buy_min_out(&via.meme_leg, quote_bridge, meme_mint, slippage)?;

    // Persistent ATAs: WSOL / stock-quote accounts are expected to live across trades.
    // Only create them when the caller opts in (CreateMissing / Auto). HotPathMinimal
    // and AssumePrepared keep create/close flags false and skip ATA ix entirely.
    // Never close intermediate ATAs — leftover dust is intentional.
    let create_quote_ata = params.create_input_mint_ata || params.create_output_mint_ata;

    let mut instructions = Vec::with_capacity(12);

    // Hops: WSOL → (currency →) quote. The first wraps WSOL; its close waits
    // for the whole route.
    let mut amount_in = sol_amount;
    for (index, (pair, (sol_hop, bridge))) in route.windows(2).zip(&bridges).enumerate() {
        let hop = build_leg_buy(
            &hop_params,
            sol_hop_as_dex_param(sol_hop),
            pair[0],
            pair[1],
            amount_in,
            index == 0 && params.create_input_mint_ata,
            false,
            create_quote_ata,
            false,
            hop_fixed_output(sol_hop, *bridge),
        )
        .await?;
        instructions.extend(hop);
        amount_in = *bridge;
    }

    // Meme leg: quote → meme. Never create/close the quote ATA on this leg.
    let meme_leg = build_leg_buy(
        params,
        meme_leg_as_dex_param(&via.meme_leg),
        quote_mint,
        meme_mint,
        quote_bridge,
        false,
        false,
        params.create_output_mint_ata,
        false,
        params.fixed_output_amount,
    )
    .await?;
    instructions.extend(meme_leg);

    if params.close_input_mint_ata {
        crate::instruction::token_account_setup::push_close_wsol_if_needed(
            &mut instructions,
            &params.payer.pubkey(),
            &wsol,
        );
    }

    Ok(instructions)
}

async fn build_sell_via_sol(
    params: &SwapParams,
    via: &StonkFunViaSolParams,
) -> Result<Vec<Instruction>> {
    if !is_native_sol(params.output_mint) {
        return Err(anyhow!(
            "StonkFunViaSol sell expects SOL/WSOL output mint, got {}",
            params.output_mint
        ));
    }
    // Selling the quote itself — what an earlier sale left behind — takes
    // only the route back to SOL, spending exactly the input.
    if let Ok(route) = sol_route_mints(via, params.input_mint) {
        let quote_amount = params
            .input_amount
            .filter(|&a| a > 0)
            .ok_or_else(|| anyhow!("StonkFunViaSol quote sale requires a non-zero amount"))?;
        let slippage = params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE);
        let create_quote_ata = params.create_input_mint_ata || params.create_output_mint_ata;
        return build_route_to_sol(params, via, &route, quote_amount, slippage, create_quote_ata)
            .await;
    }
    let meme_mint = params.input_mint;
    let quote_mint = meme_leg_quote_mint(&via.meme_leg, meme_mint)?;
    let meme_amount = params
        .input_amount
        .filter(|&a| a > 0)
        .ok_or_else(|| anyhow!("StonkFunViaSol sell requires a non-zero meme input amount"))?;
    let slippage = params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE);
    let wsol = crate::constants::WSOL_TOKEN_ACCOUNT;

    if is_native_sol(quote_mint) {
        return build_leg_sell(
            params,
            meme_leg_as_dex_param(&via.meme_leg),
            meme_mint,
            wsol,
            meme_amount,
            false,
            params.close_input_mint_ata,
            params.create_output_mint_ata,
            params.close_output_mint_ata,
            params.fixed_output_amount,
        )
        .await;
    }

    let route = sol_route_mints(via, quote_mint)?;
    let quote_bridge =
        meme_leg_sell_min_out(&via.meme_leg, meme_amount, meme_mint, slippage)?;
    if quote_bridge == 0 {
        return Err(anyhow!("StonkFunViaSol meme leg produced zero quote output"));
    }

    // Same persistence policy as buy: do not force-create or close stock quote ATAs.
    let create_quote_ata = params.create_input_mint_ata || params.create_output_mint_ata;

    let mut instructions = Vec::with_capacity(12);

    // Meme leg: meme → quote. Never close the stock quote ATA after the sale.
    let meme_leg = build_leg_sell(
        params,
        meme_leg_as_dex_param(&via.meme_leg),
        meme_mint,
        quote_mint,
        meme_amount,
        false,
        params.close_input_mint_ata,
        create_quote_ata,
        false,
        None,
    )
    .await?;
    instructions.extend(meme_leg);
    instructions.extend(
        build_route_to_sol(params, via, &route, quote_bridge, slippage, create_quote_ata).await?,
    );
    Ok(instructions)
}

/// Sell `quote_amount` of the quote along the route back to SOL: quote →
/// (currency →) WSOL, each hop selling the minimum output of the one before.
/// The last leg's WSOL create/close follows the caller's output ATA flags.
async fn build_route_to_sol(
    params: &SwapParams,
    via: &StonkFunViaSolParams,
    route: &[Pubkey],
    quote_amount: u64,
    slippage: u64,
    create_quote_ata: bool,
) -> Result<Vec<Instruction>> {
    let hops = sol_route_hops(via);
    let (hop_params, hop_slippage) = hop_swap_params(params, via, slippage);
    let mut bridges = Vec::with_capacity(hops.len());
    let mut amount = quote_amount;
    for (sol_hop, pair) in hops.iter().zip(route.windows(2)).rev() {
        let (hop, min_out) = quote_hop(sol_hop, amount, pair[1], pair[0], hop_slippage)?;
        amount = min_out;
        if amount == 0 {
            return Err(anyhow!("StonkFunViaSol SOL hop produced zero output"));
        }
        bridges.push((hop, amount));
    }

    let mut instructions = Vec::with_capacity(8);
    let mut amount_in = quote_amount;
    let last = hops.len() - 1;
    for (step, (pair, (sol_hop, bridge))) in route.windows(2).rev().zip(&bridges).enumerate() {
        let to_sol = step == last;
        let fixed_output = hop_fixed_output(sol_hop, *bridge);
        let hop = build_leg_sell(
            &hop_params,
            sol_hop_as_dex_param(sol_hop),
            pair[1],
            pair[0],
            amount_in,
            false,
            false,
            if to_sol { params.create_output_mint_ata } else { create_quote_ata },
            to_sol && params.close_output_mint_ata,
            if to_sol { params.fixed_output_amount.or(fixed_output) } else { fixed_output },
        )
        .await?;
        instructions.extend(hop);
        amount_in = *bridge;
    }
    Ok(instructions)
}

#[async_trait::async_trait]
impl InstructionBuilder for StonkFunInstructionBuilder {
    async fn build_buy_instructions(
        &self,
        params: &crate::trading::core::params::SwapParams,
    ) -> Result<Vec<Instruction>> {
        match &params.protocol_params {
            DexParamEnum::StonkFun(_) => {
                BonkInstructionBuilder.build_buy_instructions(params).await
            }
            DexParamEnum::StonkFunSwap(_) => {
                RaydiumCpmmInstructionBuilder.build_buy_instructions(params).await
            }
            DexParamEnum::StonkFunViaSol(via) => build_buy_via_sol(params, via).await,
            DexParamEnum::RaydiumCpmm(_) => {
                RaydiumCpmmInstructionBuilder.build_buy_instructions(params).await
            }
            DexParamEnum::RaydiumAmmV4(_) => {
                RaydiumAmmV4InstructionBuilder.build_buy_instructions(params).await
            }
            DexParamEnum::RaydiumClmm(_) => {
                RaydiumClmmInstructionBuilder.build_buy_instructions(params).await
            }
            DexParamEnum::OrcaWhirlpool(_) => {
                WhirlpoolInstructionBuilder.build_buy_instructions(params).await
            }
            DexParamEnum::MeteoraDlmm(_) => {
                MeteoraDlmmInstructionBuilder.build_buy_instructions(params).await
            }
            DexParamEnum::MeteoraDbc(_) => {
                MeteoraDbcInstructionBuilder.build_buy_instructions(params).await
            }
            DexParamEnum::MeteoraDammV2(_) => {
                MeteoraDammV2InstructionBuilder.build_buy_instructions(params).await
            }
            _ => Err(anyhow!("Invalid protocol params for StonkFun")),
        }
    }

    async fn build_sell_instructions(
        &self,
        params: &crate::trading::core::params::SwapParams,
    ) -> Result<Vec<Instruction>> {
        match &params.protocol_params {
            DexParamEnum::StonkFun(_) => {
                BonkInstructionBuilder.build_sell_instructions(params).await
            }
            DexParamEnum::StonkFunSwap(_) => {
                RaydiumCpmmInstructionBuilder.build_sell_instructions(params).await
            }
            DexParamEnum::StonkFunViaSol(via) => build_sell_via_sol(params, via).await,
            DexParamEnum::RaydiumCpmm(_) => {
                RaydiumCpmmInstructionBuilder.build_sell_instructions(params).await
            }
            DexParamEnum::RaydiumAmmV4(_) => {
                RaydiumAmmV4InstructionBuilder.build_sell_instructions(params).await
            }
            DexParamEnum::RaydiumClmm(_) => {
                RaydiumClmmInstructionBuilder.build_sell_instructions(params).await
            }
            DexParamEnum::OrcaWhirlpool(_) => {
                WhirlpoolInstructionBuilder.build_sell_instructions(params).await
            }
            DexParamEnum::MeteoraDlmm(_) => {
                MeteoraDlmmInstructionBuilder.build_sell_instructions(params).await
            }
            DexParamEnum::MeteoraDbc(_) => {
                MeteoraDbcInstructionBuilder.build_sell_instructions(params).await
            }
            DexParamEnum::MeteoraDammV2(_) => {
                MeteoraDammV2InstructionBuilder.build_sell_instructions(params).await
            }
            _ => Err(anyhow!("Invalid protocol params for StonkFun")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        common::GasFeeStrategy,
        instruction::utils::{
            bonk::accounts as launchlab_accounts,
            raydium_cpmm::{accounts as cpmm_accounts, SWAP_BASE_IN_DISCRIMINATOR},
        },
        swqos::TradeType,
        trading::core::params::{
            BonkParams, MeteoraDlmmParams, RaydiumAmmV4Params, RaydiumClmmParams,
            RaydiumCpmmParams, WhirlpoolParams,
        },
        utils::calc::common::calculate_min_amount_out,
    };
    use solana_sdk::{pubkey::Pubkey, signature::Keypair};
    use std::sync::Arc;

    fn pk(seed: u8) -> Pubkey {
        Pubkey::new_from_array([seed; 32])
    }

    fn curve_params(quote_mint: Pubkey) -> BonkParams {
        BonkParams {
            virtual_base: 1_000_000_000,
            virtual_quote: 30_000_000_000,
            real_base: 0,
            real_quote: 0,
            total_base_sell: 800_000_000,
            pool_state: pk(1),
            base_vault: pk(2),
            quote_vault: pk(3),
            mint_token_program: crate::constants::TOKEN_PROGRAM,
            quote_mint,
            quote_token_program: crate::constants::TOKEN_PROGRAM,
            platform_config: launchlab_accounts::STONKFUN_REWARD_PLATFORM_CONFIG,
            platform_associated_account: pk(9),
            creator_associated_account: pk(10),
            global_config: pk(11),
            curve_type: 0,
            trade_fee_rate: 2_500,
            platform_fee_rate: 10_000,
            creator_fee_rate: 0,
            base_transfer_fee: Default::default(),
            quote_transfer_fee: Default::default(),
        }
    }

    fn amm_v4_pool(coin_mint: Pubkey, pc_mint: Pubkey) -> RaydiumAmmV4Params {
        RaydiumAmmV4Params::new(
            pk(31),
            coin_mint,
            pc_mint,
            pk(32),
            pk(33),
            5_000_000_000,
            8_000_000_000,
        )
    }

    fn cpmm_pool(base_mint: Pubkey, quote_mint: Pubkey) -> RaydiumCpmmParams {
        RaydiumCpmmParams {
            pool_state: pk(21),
            amm_config: pk(22),
            base_mint,
            quote_mint,
            base_reserve: 10_000_000_000,
            quote_reserve: 20_000_000_000,
            base_vault: pk(23),
            quote_vault: pk(24),
            base_token_program: crate::constants::TOKEN_PROGRAM,
            quote_token_program: crate::constants::TOKEN_PROGRAM,
            observation_state: pk(25),
            trade_fee_rate: cpmm_accounts::TRADE_FEE_RATE,
            protocol_fee_rate: cpmm_accounts::PROTOCOL_FEE_RATE,
            fund_fee_rate: cpmm_accounts::FUND_FEE_RATE,
            creator_fee_rate: 0,
            creator_fee_on: 0,
            enable_creator_fee: false,
            base_transfer_fee: Default::default(),
            quote_transfer_fee: Default::default(),
        }
    }

    fn swap_params(
        trade_type: TradeType,
        input_mint: Pubkey,
        output_mint: Pubkey,
        protocol_params: DexParamEnum,
    ) -> SwapParams {
        SwapParams {
            rpc: None,
            payer: Arc::new(Keypair::new()),
            trade_type,
            input_mint,
            input_token_program: None,
            output_mint,
            output_token_program: None,
            input_amount: Some(1_000_000),
            slippage_basis_points: Some(100),
            address_lookup_table_accounts: Vec::new(),
            recent_blockhash: None,
            wait_tx_confirmed: false,
            protocol_params,
            open_seed_optimize: true,
            swqos_clients: Arc::new(Vec::new()),
            middleware_manager: None,
            durable_nonce: None,
            with_tip: false,
            create_input_mint_ata: true,
            close_input_mint_ata: true,
            create_output_mint_ata: true,
            close_output_mint_ata: true,
            fixed_output_amount: None,
            gas_fee_strategy: GasFeeStrategy::new(),
            simulate: true,
            log_enabled: false,
            wait_for_all_submits: false,
            use_dedicated_sender_threads: false,
            sender_thread_cores: None,
            max_sender_concurrency: 0,
            effective_core_ids: Arc::new(Vec::new()),
            check_min_tip: false,
            transaction_version: crate::common::TradeTransactionVersion::V0,
            grpc_recv_us: None,
            use_exact_sol_amount: None,
            precheck: None,
        }
    }

    #[tokio::test]
    async fn via_sol_curve_buy_composes_sol_hop_then_launchlab() {
        let stock = pk(40);
        let meme = pk(41);
        let via = StonkFunViaSolParams::curve(
            curve_params(stock),
            StonkFunSolHop::RaydiumCpmm(cpmm_pool(
                crate::constants::WSOL_TOKEN_ACCOUNT,
                stock,
            )),
        );
        let params = swap_params(
            TradeType::Buy,
            crate::constants::WSOL_TOKEN_ACCOUNT,
            meme,
            DexParamEnum::StonkFunViaSol(via),
        );

        let ixs = StonkFunInstructionBuilder
            .build_buy_instructions(&params)
            .await
            .expect("compose curve via-sol buy");
        assert!(ixs.len() >= 2);

        let programs: Vec<_> = ixs.iter().map(|ix| ix.program_id).collect();
        assert!(programs.contains(&cpmm_accounts::RAYDIUM_CPMM));
        assert!(programs.contains(&launchlab_accounts::BONK));

        let cpmm_ix = ixs.iter().find(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM).unwrap();
        assert_eq!(&cpmm_ix.data[..8], SWAP_BASE_IN_DISCRIMINATOR);
        assert_eq!(cpmm_ix.accounts[10].pubkey, crate::constants::WSOL_TOKEN_ACCOUNT);
        assert_eq!(cpmm_ix.accounts[11].pubkey, stock);

        let curve_ix = ixs.iter().find(|ix| ix.program_id == launchlab_accounts::BONK).unwrap();
        assert_eq!(curve_ix.accounts[9].pubkey, meme);
        assert_eq!(curve_ix.accounts[10].pubkey, stock);
        let curve_quote_in = u64::from_le_bytes(curve_ix.data[8..16].try_into().unwrap());
        let hop1_min_out = u64::from_le_bytes(cpmm_ix.data[16..24].try_into().unwrap());
        assert_eq!(curve_quote_in, hop1_min_out);
        assert!(curve_quote_in > 0);
    }

    #[tokio::test]
    async fn via_sol_hop_slippage_sets_the_bridge_apart_from_the_meme_leg() {
        let stock = pk(40);
        let meme = pk(41);
        let quote_in = |hop_slippage: Option<u64>| async move {
            let mut via = StonkFunViaSolParams::curve(
                curve_params(stock),
                StonkFunSolHop::RaydiumCpmm(cpmm_pool(crate::constants::WSOL_TOKEN_ACCOUNT, stock)),
            );
            if let Some(bps) = hop_slippage {
                via = via.with_hop_slippage_basis_points(bps);
            }
            let mut params = swap_params(
                TradeType::Buy,
                crate::constants::WSOL_TOKEN_ACCOUNT,
                meme,
                DexParamEnum::StonkFunViaSol(via),
            );
            params.slippage_basis_points = Some(1_500);
            let ixs = StonkFunInstructionBuilder.build_buy_instructions(&params).await.unwrap();
            let hop = ixs.iter().find(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM).unwrap();
            let curve = ixs.iter().find(|ix| ix.program_id == launchlab_accounts::BONK).unwrap();
            let hop_min_out = u64::from_le_bytes(hop.data[16..24].try_into().unwrap());
            let curve_quote_in = u64::from_le_bytes(curve.data[8..16].try_into().unwrap());
            assert_eq!(curve_quote_in, hop_min_out);
            curve_quote_in
        };
        let trade_slippage = quote_in(None).await;
        let hop_slippage = quote_in(Some(50)).await;
        // 0.5% instead of 15% off the hop's expected output.
        let ratio = trade_slippage as f64 / hop_slippage as f64;
        assert!((ratio - 8_500.0 / 9_950.0).abs() < 1e-3, "ratio {ratio}");
    }

    #[tokio::test]
    async fn via_sol_graduated_sell_composes_meme_leg_then_sol_hop() {
        let stock = pk(50);
        let meme = pk(51);
        let via = StonkFunViaSolParams::graduated(
            cpmm_pool(stock, meme),
            StonkFunSolHop::RaydiumCpmm(cpmm_pool(
                crate::constants::WSOL_TOKEN_ACCOUNT,
                stock,
            )),
        );
        let params = swap_params(
            TradeType::Sell,
            meme,
            crate::constants::WSOL_TOKEN_ACCOUNT,
            DexParamEnum::StonkFunViaSol(via),
        );

        let ixs = StonkFunInstructionBuilder
            .build_sell_instructions(&params)
            .await
            .expect("compose graduated via-sol sell");
        assert!(ixs.len() >= 2);

        let cpmm_ixs: Vec<_> =
            ixs.iter().filter(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM).collect();
        assert_eq!(cpmm_ixs.len(), 2);

        assert_eq!(cpmm_ixs[0].accounts[10].pubkey, meme);
        assert_eq!(cpmm_ixs[0].accounts[11].pubkey, stock);
        assert_eq!(cpmm_ixs[1].accounts[10].pubkey, stock);
        assert_eq!(cpmm_ixs[1].accounts[11].pubkey, crate::constants::WSOL_TOKEN_ACCOUNT);

        let hop1_min_out = u64::from_le_bytes(cpmm_ixs[0].data[16..24].try_into().unwrap());
        let hop2_amount_in = u64::from_le_bytes(cpmm_ixs[1].data[8..16].try_into().unwrap());
        assert_eq!(hop1_min_out, hop2_amount_in);
        assert!(hop2_amount_in > 0);
    }

    #[tokio::test]
    async fn via_sol_rejects_mismatched_sol_hop_pool() {
        let stock = pk(60);
        let other = pk(61);
        let meme = pk(62);
        let via = StonkFunViaSolParams::curve(
            curve_params(stock),
            StonkFunSolHop::RaydiumCpmm(cpmm_pool(
                crate::constants::WSOL_TOKEN_ACCOUNT,
                other,
            )),
        );
        let params = swap_params(
            TradeType::Buy,
            crate::constants::WSOL_TOKEN_ACCOUNT,
            meme,
            DexParamEnum::StonkFunViaSol(via),
        );
        let err = StonkFunInstructionBuilder.build_buy_instructions(&params).await.unwrap_err();
        assert!(err.to_string().contains("does not match WSOL"));
    }

    #[tokio::test]
    async fn via_sol_hot_path_skips_ata_create_and_never_closes_quote() {
        let stock = pk(70);
        let meme = pk(71);
        let via = StonkFunViaSolParams::curve(
            curve_params(stock),
            StonkFunSolHop::RaydiumCpmm(cpmm_pool(
                crate::constants::WSOL_TOKEN_ACCOUNT,
                stock,
            )),
        );
        let mut params = swap_params(
            TradeType::Buy,
            crate::constants::WSOL_TOKEN_ACCOUNT,
            meme,
            DexParamEnum::StonkFunViaSol(via),
        );
        // HotPathMinimal / AssumePrepared: ATAs are prepared offline and reused.
        params.create_input_mint_ata = false;
        params.close_input_mint_ata = false;
        params.create_output_mint_ata = false;
        params.close_output_mint_ata = false;

        let ixs = StonkFunInstructionBuilder
            .build_buy_instructions(&params)
            .await
            .expect("hot-path via-sol buy");

        // Only the two swap program instructions — no ATA create / close / wrap.
        assert_eq!(ixs.len(), 2);
        assert_eq!(ixs[0].program_id, cpmm_accounts::RAYDIUM_CPMM);
        assert_eq!(ixs[1].program_id, launchlab_accounts::BONK);
    }

    #[tokio::test]
    async fn via_sol_curve_buy_composes_amm_v4_sol_hop() {
        let stock = pk(80);
        let meme = pk(81);
        let via = StonkFunViaSolParams::curve_with_amm_v4(
            curve_params(stock),
            amm_v4_pool(crate::constants::WSOL_TOKEN_ACCOUNT, stock),
        );
        let params = swap_params(
            TradeType::Buy,
            crate::constants::WSOL_TOKEN_ACCOUNT,
            meme,
            DexParamEnum::StonkFunViaSol(via),
        );

        let ixs = StonkFunInstructionBuilder
            .build_buy_instructions(&params)
            .await
            .expect("compose curve via-sol buy with amm v4 hop");
        let programs: Vec<_> = ixs.iter().map(|ix| ix.program_id).collect();
        assert!(programs
            .contains(&crate::instruction::utils::raydium_amm_v4::accounts::RAYDIUM_AMM_V4));
        assert!(programs.contains(&launchlab_accounts::BONK));
    }

    fn spot(price: f64) -> HopSpot {
        HopSpot { price, fee: 0.003 }
    }

    fn whirlpool_pool(mint_a: Pubkey, mint_b: Pubkey, spot: Option<HopSpot>) -> WhirlpoolParams {
        let pool = WhirlpoolParams::new(
            pk(91),
            mint_a,
            mint_b,
            pk(92),
            pk(93),
            crate::constants::TOKEN_PROGRAM,
            crate::constants::TOKEN_PROGRAM,
            vec![pk(94), pk(95), pk(96)],
        );
        match spot {
            Some(spot) => pool.with_spot(spot),
            None => pool,
        }
    }

    /// A CLMM pool at `price` (raw token 1 per token 0), loaded for exact quotes,
    /// with one position from the start of the current tick array to the start
    /// of the next.
    fn clmm_pool(mint_0: Pubkey, mint_1: Pubkey, price: f64) -> RaydiumClmmParams {
        use crate::trading::core::params::{ClmmQuoteState, TokenTransferFee};
        use crate::utils::calc::raydium_clmm::{
            config::AmmConfig,
            state::{DynamicFeeInfo, PoolState, TickArrayState, TickState},
            tick_math,
        };
        let (pool, tick_spacing, liquidity) = (pk(102), 60u16, 10u128.pow(15));
        let sqrt_price_x64 = (price.sqrt() * 2f64.powi(64)) as u128;
        let tick_current = tick_math::get_tick_at_sqrt_price(sqrt_price_x64).unwrap();
        let ticks_per_array = TickArrayState::tick_count(tick_spacing);
        let start = TickArrayState::get_array_start_index(tick_current, tick_spacing);
        let mut tick_array_bitmap = [0u64; 16];
        let arrays: Vec<TickArrayState> = [start, start + ticks_per_array]
            .into_iter()
            .map(|array_start| {
                let bit = (array_start / ticks_per_array + 512) as usize;
                tick_array_bitmap[bit / 64] |= 1 << (bit % 64);
                let mut ticks: Vec<TickState> = (0..60)
                    .map(|i| TickState {
                        tick: array_start + i * i32::from(tick_spacing),
                        ..TickState::default()
                    })
                    .collect();
                // Lower bound at this array's start, upper bound at the next one's.
                let net =
                    if array_start == start { liquidity as i128 } else { -(liquidity as i128) };
                ticks[0].liquidity_gross = liquidity;
                ticks[0].liquidity_net = net;
                TickArrayState {
                    pool_id: pool,
                    start_tick_index: array_start,
                    ticks,
                    initialized_tick_count: 1,
                }
            })
            .collect();
        let state = PoolState {
            amm_config: pk(101),
            token_mint_0: mint_0,
            token_mint_1: mint_1,
            token_vault_0: pk(104),
            token_vault_1: pk(105),
            observation_key: pk(103),
            tick_spacing,
            liquidity,
            sqrt_price_x64,
            tick_current,
            status: 0,
            fee_on: 0,
            tick_array_bitmap,
            open_time: 0,
            dynamic_fee_info: DynamicFeeInfo::default(),
        };
        let mut params = RaydiumClmmParams::new(
            pk(101),
            pool,
            pk(103),
            mint_0,
            mint_1,
            pk(104),
            pk(105),
            crate::constants::TOKEN_PROGRAM,
            crate::constants::TOKEN_PROGRAM,
            Vec::new(),
        );
        params.quote_state = Some(Box::new(ClmmQuoteState {
            pool: state,
            config: AmmConfig {
                protocol_fee_rate: 120_000,
                trade_fee_rate: 2_500,
                tick_spacing,
                fund_fee_rate: 40_000,
            },
            tick_arrays: arrays,
            bitmap_extension: None,
            token_0_transfer_fee: TokenTransferFee::default(),
            token_1_transfer_fee: TokenTransferFee::default(),
            unix_timestamp: 1,
        }));
        params
    }

    fn dlmm_pair(mint_x: Pubkey, mint_y: Pubkey, price: f64) -> MeteoraDlmmParams {
        MeteoraDlmmParams::new(
            pk(111),
            pk(112),
            pk(113),
            mint_x,
            mint_y,
            pk(114),
            crate::constants::TOKEN_PROGRAM,
            crate::constants::TOKEN_PROGRAM,
            vec![pk(115)],
        )
        .with_spot(spot(price))
    }

    /// Input amount and minimum output of a swap instruction.
    fn amounts(ix: &Instruction) -> (u64, u64) {
        (
            u64::from_le_bytes(ix.data[8..16].try_into().unwrap()),
            u64::from_le_bytes(ix.data[16..24].try_into().unwrap()),
        )
    }

    fn only(ixs: &[Instruction], program: Pubkey) -> &Instruction {
        let mut found = ixs.iter().filter(|ix| ix.program_id == program);
        let ix = found.next().expect("instruction of the program");
        assert!(found.next().is_none(), "one instruction of the program");
        ix
    }

    #[tokio::test]
    async fn via_sol_curve_buy_through_a_whirlpool_spends_its_spot_min_out() {
        use crate::instruction::utils::whirlpool::PROGRAM_ID as WHIRLPOOL;
        let (stock, meme) = (pk(120), pk(121));
        let wsol = crate::constants::WSOL_TOKEN_ACCOUNT;
        let via = StonkFunViaSolParams::curve(
            curve_params(stock),
            whirlpool_pool(wsol, stock, Some(spot(50.0))),
        )
        .with_hop_slippage_basis_points(100);
        let params = swap_params(TradeType::Buy, wsol, meme, DexParamEnum::StonkFunViaSol(via));

        let ixs = StonkFunInstructionBuilder.build_buy_instructions(&params).await.unwrap();
        let hop = only(&ixs, WHIRLPOOL);
        let expected = spot(50.0).min_amount_out(1_000_000, true, 100);
        assert_eq!(amounts(hop), (1_000_000, expected));
        // Exact in, A (WSOL) to B.
        assert_eq!(&hop.data[40..42], &[1, 1]);
        let curve = only(&ixs, launchlab_accounts::BONK);
        assert_eq!(amounts(curve).0, expected);
        let order: Vec<_> = ixs.iter().map(|ix| ix.program_id).collect();
        let hop_at = order.iter().position(|program| *program == WHIRLPOOL).unwrap();
        let curve_at = order.iter().position(|program| *program == launchlab_accounts::BONK).unwrap();
        assert!(hop_at < curve_at);
    }

    #[tokio::test]
    async fn via_sol_two_hop_buy_chains_min_outs_through_a_currency() {
        use crate::instruction::utils::{
            meteora_dlmm::PROGRAM_ID as DLMM, raydium_clmm::PROGRAM_ID as CLMM,
        };
        let (usdc, stock, meme) = (pk(130), pk(131), pk(132));
        let wsol = crate::constants::WSOL_TOKEN_ACCOUNT;
        // SOL → USDC on DLMM, then USDC → stock on CLMM, where the stock is token 0.
        let clmm = clmm_pool(stock, usdc, 3.0);
        let via = StonkFunViaSolParams::curve(curve_params(stock), dlmm_pair(wsol, usdc, 0.2))
            .with_quote_hop(clmm.clone())
            .with_hop_slippage_basis_points(100);
        let params = swap_params(TradeType::Buy, wsol, meme, DexParamEnum::StonkFunViaSol(via));

        let ixs = StonkFunInstructionBuilder.build_buy_instructions(&params).await.unwrap();
        let usdc_out = spot(0.2).min_amount_out(1_000_000, true, 100);
        // The CLMM hop is quoted exactly and reads the tick arrays that quote crosses.
        let clmm_quote = clmm.quote_exact_in(&usdc, usdc_out).unwrap();
        let stock_out = calculate_min_amount_out(clmm_quote.amount_out, 100);
        assert_eq!(amounts(only(&ixs, DLMM)), (1_000_000, usdc_out));
        let clmm_ix = only(&ixs, CLMM);
        assert_eq!(amounts(clmm_ix), (usdc_out, stock_out));
        let clmm_arrays: Vec<Pubkey> =
            clmm_ix.accounts[13..].iter().map(|meta| meta.pubkey).collect();
        assert_eq!(clmm_arrays, clmm_quote.tick_arrays);
        assert_eq!(amounts(only(&ixs, launchlab_accounts::BONK)).0, stock_out);
        let order: Vec<_> = ixs
            .iter()
            .map(|ix| ix.program_id)
            .filter(|program| [DLMM, CLMM, launchlab_accounts::BONK].contains(program))
            .collect();
        assert_eq!(order, [DLMM, CLMM, launchlab_accounts::BONK]);
    }

    #[tokio::test]
    async fn via_sol_buy_through_an_exactly_quoted_dlmm_pair_reads_the_bin_arrays_it_walks() {
        use crate::instruction::utils::meteora_dlmm::PROGRAM_ID as DLMM;
        use crate::trading::core::params::dlmm_fixture_pair;
        // A SOL → USDC swap captured on mainnet: 79 bins over three bin arrays,
        // limit orders filled on the way.
        let (pair, lamports, usdc_credited) =
            dlmm_fixture_pair("3D9MyL5iD9uqbe2FYvivHsywHWr3nJR1krEcEWXUCGzr-x2y-536870912000.json");
        let (wsol, usdc, meme) = (crate::constants::WSOL_TOKEN_ACCOUNT, pair.token_y_mint, pk(150));
        assert_eq!(pair.token_x_mint, wsol);
        let quote = pair.quote_exact_in(&wsol, lamports).unwrap();
        assert_eq!(quote.amount_out, usdc_credited);
        assert_eq!(
            StonkFunSolHop::MeteoraDlmm(pair.clone()).quote_exact_in(&wsol, lamports).unwrap(),
            usdc_credited
        );
        let via = StonkFunViaSolParams::curve(curve_params(usdc), pair)
            .with_hop_slippage_basis_points(100);
        let mut params = swap_params(TradeType::Buy, wsol, meme, DexParamEnum::StonkFunViaSol(via));
        params.input_amount = Some(lamports);

        let ixs = StonkFunInstructionBuilder.build_buy_instructions(&params).await.unwrap();
        let hop = only(&ixs, DLMM);
        let usdc_out = calculate_min_amount_out(usdc_credited, 100);
        assert_eq!(amounts(hop), (lamports, usdc_out));
        // After swap2's 16 accounts come the bin arrays the quote walks.
        let arrays: Vec<Pubkey> = hop.accounts[16..].iter().map(|meta| meta.pubkey).collect();
        assert_eq!(arrays, quote.bin_arrays);
        assert_eq!(amounts(only(&ixs, launchlab_accounts::BONK)).0, usdc_out);
    }

    #[tokio::test]
    async fn via_sol_graduated_sell_through_a_whirlpool_cashes_out_the_meme_legs_min_out() {
        use crate::instruction::utils::whirlpool::PROGRAM_ID as WHIRLPOOL;
        let (stock, meme) = (pk(140), pk(141));
        let wsol = crate::constants::WSOL_TOKEN_ACCOUNT;
        let via = StonkFunViaSolParams::graduated(
            cpmm_pool(stock, meme),
            whirlpool_pool(stock, wsol, Some(spot(0.02))),
        );
        let params = swap_params(TradeType::Sell, meme, wsol, DexParamEnum::StonkFunViaSol(via));

        let ixs = StonkFunInstructionBuilder.build_sell_instructions(&params).await.unwrap();
        let (meme_in, quote_bridge) = amounts(only(&ixs, cpmm_accounts::RAYDIUM_CPMM));
        assert_eq!(meme_in, 1_000_000);
        let hop = only(&ixs, WHIRLPOOL);
        // The trade's 1% slippage covers the hop too.
        assert_eq!(amounts(hop), (quote_bridge, spot(0.02).min_amount_out(quote_bridge, true, 100)));
    }

    #[tokio::test]
    async fn via_sol_quote_sale_takes_only_the_route_back_to_sol() {
        use crate::instruction::utils::{
            meteora_dlmm::PROGRAM_ID as DLMM, raydium_clmm::PROGRAM_ID as CLMM,
        };
        let (usdc, stock) = (pk(160), pk(161));
        let wsol = crate::constants::WSOL_TOKEN_ACCOUNT;
        let clmm = clmm_pool(stock, usdc, 3.0);
        let via = StonkFunViaSolParams::quote_sale(dlmm_pair(wsol, usdc, 0.2))
            .with_quote_hop(clmm.clone())
            .with_hop_slippage_basis_points(100);
        // What an earlier sale left behind: the stock itself goes back to SOL.
        let params = swap_params(TradeType::Sell, stock, wsol, DexParamEnum::StonkFunViaSol(via));

        let ixs = StonkFunInstructionBuilder.build_sell_instructions(&params).await.unwrap();
        assert!(ixs.iter().all(|ix| ix.program_id != launchlab_accounts::BONK));
        let clmm_quote = clmm.quote_exact_in(&stock, 1_000_000).unwrap();
        let usdc_out = calculate_min_amount_out(clmm_quote.amount_out, 100);
        assert_eq!(amounts(only(&ixs, CLMM)), (1_000_000, usdc_out));
        assert_eq!(amounts(only(&ixs, DLMM)).0, usdc_out);
        let order: Vec<_> = ixs
            .iter()
            .map(|ix| ix.program_id)
            .filter(|program| [DLMM, CLMM].contains(program))
            .collect();
        assert_eq!(order, [CLMM, DLMM]);
    }

    #[tokio::test]
    async fn via_sol_concentrated_hop_needs_its_spot_price() {
        let (stock, meme) = (pk(150), pk(151));
        let wsol = crate::constants::WSOL_TOKEN_ACCOUNT;
        let via = StonkFunViaSolParams::curve(curve_params(stock), whirlpool_pool(wsol, stock, None));
        let params = swap_params(TradeType::Buy, wsol, meme, DexParamEnum::StonkFunViaSol(via));
        let err = StonkFunInstructionBuilder.build_buy_instructions(&params).await.unwrap_err();
        assert!(err.to_string().contains("spot price"), "{err}");
    }

    #[tokio::test]
    async fn via_sol_rejects_a_quote_hop_that_misses_the_quote() {
        let (usdc, stock, other, meme) = (pk(160), pk(161), pk(162), pk(163));
        let wsol = crate::constants::WSOL_TOKEN_ACCOUNT;
        for quote_hop in [clmm_pool(usdc, other, 1.0), clmm_pool(wsol, stock, 1.0)] {
            let via = StonkFunViaSolParams::curve(curve_params(stock), dlmm_pair(wsol, usdc, 0.2))
                .with_quote_hop(quote_hop);
            let params = swap_params(TradeType::Buy, wsol, meme, DexParamEnum::StonkFunViaSol(via));
            let err = StonkFunInstructionBuilder.build_buy_instructions(&params).await.unwrap_err();
            assert!(err.to_string().contains("does not match"), "{err}");
        }
    }

    #[test]
    fn calculate_min_amount_out_is_used_for_bridge_matching() {
        assert_eq!(calculate_min_amount_out(10_000, 100), 9_900);
    }

    #[test]
    fn sol_hops_quote_what_their_pools_pay() {
        let (quote, other) = (pk(170), pk(171));
        let wsol = crate::constants::WSOL_TOKEN_ACCOUNT;
        let cpmm = cpmm_pool(wsol, quote);
        assert_eq!(
            StonkFunSolHop::RaydiumCpmm(cpmm.clone()).quote_exact_in(&wsol, 1_000_000).unwrap(),
            compute_swap_amount_for_pool(&cpmm, true, 1_000_000, 0).unwrap().amount_out
        );
        let amm_v4 = amm_v4_pool(quote, wsol);
        let sol = crate::constants::SOL_TOKEN_ACCOUNT;
        assert_eq!(
            StonkFunSolHop::RaydiumAmmV4(amm_v4.clone()).quote_exact_in(&sol, 1_000_000).unwrap(),
            compute_amm_v4_swap_amount_for_pool(&amm_v4, false, 1_000_000, 0).unwrap().amount_out
        );
        let clmm = clmm_pool(wsol, quote, 2.0);
        assert_eq!(
            StonkFunSolHop::RaydiumClmm(clmm.clone()).quote_exact_in(&quote, 1_000_000).unwrap(),
            clmm.quote_exact_in(&quote, 1_000_000).unwrap().amount_out
        );
        assert!(StonkFunSolHop::RaydiumCpmm(cpmm).quote_exact_in(&other, 1_000_000).is_err());
        let whirlpool = StonkFunSolHop::OrcaWhirlpool(whirlpool_pool(wsol, quote, Some(spot(2.0))));
        assert!(whirlpool.quote_exact_in(&wsol, 1_000_000).is_err());
    }

    // ---- Meteora DBC and DAMM v2 meme legs ----

    const USDC: Pubkey = crate::constants::USDC_TOKEN_ACCOUNT;

    /// The curve of mainnet pool 2Rz8zRLAqMtXKBGsxb8DwYN1Ed13TDwLtxNUrEUHtBJY.
    fn dbc_pool(meme: Pubkey, quote_mint: Pubkey) -> MeteoraDbcParams {
        use crate::instruction::utils::meteora_dbc_types::{DbcBaseFee, DbcConfig, DbcCurvePoint};
        use crate::trading::core::params::DbcQuoteState;
        MeteoraDbcParams::new(
            pk(50),
            pk(51),
            meme,
            quote_mint,
            pk(52),
            pk(53),
            crate::constants::TOKEN_PROGRAM,
            crate::constants::TOKEN_PROGRAM,
        )
        .with_quote(DbcQuoteState {
            config: Arc::new(DbcConfig {
                base_fee: DbcBaseFee { cliff_fee_numerator: 20_000_000, ..Default::default() },
                migration_sqrt_price: 1_750_011_800_614_054_764,
                sqrt_start_price: 583_337_266_871_351_588,
                curve: vec![DbcCurvePoint {
                    sqrt_price: 1_837_512_390_644_757_503,
                    liquidity: 2_916_686_334_356_757_942_357_946_112_045,
                }],
                ..Default::default()
            }),
            sqrt_price: 1_730_409_438_693_799_042,
            fee_numerator: 20_000_000,
            rate_limited_buys: false,
        })
    }

    /// The state of mainnet pool EPy3Rnwz9G1eg1wx6a9wCoEsnSFCwb3r4keFFzxauLLX.
    fn damm_v2_pool(token_a_mint: Pubkey, token_b_mint: Pubkey) -> MeteoraDammV2Params {
        use crate::utils::calc::meteora_damm_v2::DammV2QuoteState;
        MeteoraDammV2Params::new(
            pk(60),
            pk(61),
            pk(62),
            token_a_mint,
            token_b_mint,
            crate::constants::TOKEN_PROGRAM,
            crate::constants::TOKEN_PROGRAM,
        )
        .with_quote(DammV2QuoteState {
            sqrt_price: 2_711_494_429_538_635_068,
            liquidity: 582_338_177_451_872_515_801_881_719_705,
            sqrt_min_price: 4_295_048_016,
            sqrt_max_price: 79_226_673_521_066_979_257_578_248_091,
            fee_numerator: 20_000_000,
            collect_fee_mode: 1,
        })
    }

    fn u64_at(data: &[u8], offset: usize) -> u64 {
        u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
    }

    fn usdc_hop() -> StonkFunSolHop {
        StonkFunSolHop::RaydiumCpmm(cpmm_pool(crate::constants::WSOL_TOKEN_ACCOUNT, USDC))
    }

    #[tokio::test]
    async fn via_sol_meteora_dbc_buy_spends_the_hops_minimum_output() {
        use crate::instruction::utils::meteora_dbc::accounts as dbc_accounts;
        let meme = pk(41);
        let pool = dbc_pool(meme, USDC);
        let curve = pool.quote.clone().unwrap();
        let via = StonkFunViaSolParams::meteora_dbc(pool, usdc_hop());
        let params = swap_params(
            TradeType::Buy,
            crate::constants::WSOL_TOKEN_ACCOUNT,
            meme,
            DexParamEnum::StonkFunViaSol(via),
        );
        let ixs = StonkFunInstructionBuilder.build_buy_instructions(&params).await.unwrap();

        let hop = ixs.iter().position(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM).unwrap();
        let leg = ixs.iter().position(|ix| ix.program_id == dbc_accounts::METEORA_DBC).unwrap();
        assert!(hop < leg);
        // The curve leg buys with the hop's minimum output, and asks for its
        // own quote on the curve less the trade's slippage.
        let bridge = u64_at(&ixs[hop].data, 16);
        assert_eq!(u64_at(&ixs[leg].data, 8), bridge);
        let quoted = curve.quote_exact_in(true, bridge).unwrap().amount_out;
        assert_eq!(u64_at(&ixs[leg].data, 16), calculate_min_amount_out(quoted, 100));
        assert_eq!(ixs[leg].accounts[7].pubkey, meme);
        assert_eq!(ixs[leg].accounts[8].pubkey, USDC);
    }

    #[tokio::test]
    async fn meteora_dbc_pool_priced_in_sol_needs_no_hop() {
        use crate::instruction::utils::meteora_dbc::accounts as dbc_accounts;
        let meme = pk(41);
        let pool = dbc_pool(meme, crate::constants::WSOL_TOKEN_ACCOUNT);
        // Alone, or behind a route it does not use.
        let via = StonkFunViaSolParams::meteora_dbc(pool.clone(), usdc_hop());
        for protocol_params in [DexParamEnum::MeteoraDbc(pool), DexParamEnum::StonkFunViaSol(via)] {
            let params = swap_params(
                TradeType::Buy,
                crate::constants::WSOL_TOKEN_ACCOUNT,
                meme,
                protocol_params,
            );
            let ixs = StonkFunInstructionBuilder.build_buy_instructions(&params).await.unwrap();
            assert!(ixs.iter().all(|ix| ix.program_id != cpmm_accounts::RAYDIUM_CPMM));
            let leg = ixs.iter().find(|ix| ix.program_id == dbc_accounts::METEORA_DBC).unwrap();
            assert_eq!(u64_at(&leg.data, 8), 1_000_000);
            assert_eq!(leg.accounts[8].pubkey, crate::constants::WSOL_TOKEN_ACCOUNT);
        }
    }

    #[tokio::test]
    async fn via_sol_meteora_damm_v2_buy_quotes_the_migrated_pool() {
        use crate::instruction::utils::meteora_damm_v2::accounts as damm_accounts;
        use crate::utils::calc::meteora_damm_v2::quote_exact_in;
        let meme = pk(41);
        let pool = damm_v2_pool(meme, USDC);
        let state = pool.quote.unwrap();
        let via = StonkFunViaSolParams::meteora_damm_v2(pool, usdc_hop());
        let params = swap_params(
            TradeType::Buy,
            crate::constants::WSOL_TOKEN_ACCOUNT,
            meme,
            DexParamEnum::StonkFunViaSol(via),
        );
        let ixs = StonkFunInstructionBuilder.build_buy_instructions(&params).await.unwrap();
        let hop = ixs.iter().find(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM).unwrap();
        let leg = ixs.iter().find(|ix| ix.program_id == damm_accounts::METEORA_DAMM_V2).unwrap();
        let bridge = u64_at(&hop.data, 16);
        assert_eq!(u64_at(&leg.data, 8), bridge);
        // USDC is token B: the buy goes B to A.
        let quoted = quote_exact_in(&state, false, bridge).unwrap().amount_out;
        assert_eq!(u64_at(&leg.data, 16), calculate_min_amount_out(quoted, 100));
    }

    #[tokio::test]
    async fn meteora_damm_v2_sale_pays_the_pools_other_side() {
        use crate::instruction::utils::meteora_damm_v2::accounts as damm_accounts;
        use crate::utils::calc::meteora_damm_v2::quote_exact_in;
        let meme = pk(41);
        // A quote that is neither WSOL nor USDC, with the token as token B.
        let quote_mint = pk(42);
        let mut pool = damm_v2_pool(quote_mint, meme);
        pool.token_a_program = crate::constants::TOKEN_PROGRAM_2022;
        let state = pool.quote.unwrap();
        let mut params =
            swap_params(TradeType::Sell, meme, quote_mint, DexParamEnum::MeteoraDammV2(pool));
        params.open_seed_optimize = false;
        params.create_output_mint_ata = false;
        params.close_output_mint_ata = false;
        params.close_input_mint_ata = false;
        let payer = params.payer.pubkey();
        let ixs = StonkFunInstructionBuilder.build_sell_instructions(&params).await.unwrap();
        assert_eq!(ixs.len(), 1);
        let leg = &ixs[0];
        assert_eq!(leg.program_id, damm_accounts::METEORA_DAMM_V2);
        let ata = |mint: &Pubkey, program: &Pubkey| {
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &payer, mint, program, false,
            )
        };
        assert_eq!(leg.accounts[2].pubkey, ata(&meme, &crate::constants::TOKEN_PROGRAM));
        assert_eq!(leg.accounts[3].pubkey, ata(&quote_mint, &crate::constants::TOKEN_PROGRAM_2022));
        // The token is B: the sale goes B to A.
        let quoted = quote_exact_in(&state, false, 1_000_000).unwrap().amount_out;
        assert_eq!(u64_at(&leg.data, 16), calculate_min_amount_out(quoted, 100));
    }

    #[tokio::test]
    async fn via_sol_meteora_legs_without_their_state_are_errors() {
        let meme = pk(41);
        let mut dbc = dbc_pool(meme, USDC);
        dbc.quote = None;
        let mut damm = damm_v2_pool(meme, USDC);
        damm.quote = None;
        // A DAMM v2 pool that does not hold the token has no quote side.
        let other = damm_v2_pool(pk(70), USDC);
        for via in [
            StonkFunViaSolParams::meteora_dbc(dbc, usdc_hop()),
            StonkFunViaSolParams::meteora_damm_v2(damm, usdc_hop()),
            StonkFunViaSolParams::meteora_damm_v2(other, usdc_hop()),
        ] {
            let params = swap_params(
                TradeType::Buy,
                crate::constants::WSOL_TOKEN_ACCOUNT,
                meme,
                DexParamEnum::StonkFunViaSol(via),
            );
            assert!(StonkFunInstructionBuilder.build_buy_instructions(&params).await.is_err());
        }
    }
}
