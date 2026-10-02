//! StonkFun trading across its LaunchLab curve and graduated CPMM pools.
//!
//! Also supports atomic SOL ↔ quote ↔ meme routing via
//! [`DexParamEnum::StonkFunViaSol`] so wallets do not need to pre-hold stock
//! quote tokens.

use super::{
    bonk::BonkInstructionBuilder, raydium_amm_v4::RaydiumAmmV4InstructionBuilder,
    raydium_cpmm::RaydiumCpmmInstructionBuilder,
};
use crate::{
    constants::trade::trade::DEFAULT_SLIPPAGE,
    trading::core::{
        params::{
            DexParamEnum, RaydiumAmmV4Params, RaydiumCpmmParams, StonkFunMemeLeg, StonkFunSolHop,
            StonkFunViaSolParams, SwapParams,
        },
        traits::InstructionBuilder,
    },
    utils::calc::{
        bonk::{get_buy_quote, get_sell_min_amount_out},
        raydium_amm_v4::compute_swap_amount_for_pool as compute_amm_v4_swap_amount,
        raydium_cpmm::compute_swap_amount_for_pool,
    },
};
use anyhow::{anyhow, Result};
use solana_sdk::{instruction::Instruction, pubkey::Pubkey, signer::Signer};

/// User-facing StonkFun builder that selects the curve, graduated swap, or
/// SOL-routed two-hop path from the protocol params variant.
pub struct StonkFunInstructionBuilder;

pub(super) fn normalize_native_sol(mint: Pubkey) -> Pubkey {
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
    if params.global_config == crate::instruction::utils::bonk::accounts::USD1_GLOBAL_CONFIG {
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

fn meme_leg_quote_mint(meme_leg: &StonkFunMemeLeg, meme_mint: Pubkey) -> Result<Pubkey> {
    match meme_leg {
        StonkFunMemeLeg::Curve(params) => curve_quote_mint(params),
        StonkFunMemeLeg::Graduated(params) => graduated_quote_mint(params, meme_mint),
    }
}

fn meme_leg_quote_program(meme_leg: &StonkFunMemeLeg, quote_mint: Pubkey) -> Pubkey {
    match meme_leg {
        StonkFunMemeLeg::Curve(p) => {
            if p.quote_token_program == Pubkey::default() {
                crate::constants::TOKEN_PROGRAM
            } else {
                p.quote_token_program
            }
        }
        StonkFunMemeLeg::Graduated(p) => {
            if p.base_mint == quote_mint {
                p.base_token_program
            } else {
                p.quote_token_program
            }
        }
    }
}

fn meme_leg_as_dex_param(meme_leg: &StonkFunMemeLeg) -> DexParamEnum {
    match meme_leg {
        StonkFunMemeLeg::Curve(params) => DexParamEnum::StonkFun(params.clone()),
        StonkFunMemeLeg::Graduated(params) => DexParamEnum::StonkFunSwap(params.clone()),
    }
}

fn sol_hop_as_dex_param(sol_hop: &StonkFunSolHop) -> DexParamEnum {
    match sol_hop {
        StonkFunSolHop::RaydiumCpmm(params) => DexParamEnum::RaydiumCpmm(params.clone()),
        StonkFunSolHop::RaydiumAmmV4(params) => DexParamEnum::RaydiumAmmV4(params.clone()),
        StonkFunSolHop::Route(route) => DexParamEnum::StonkFunQuoteRoute(route.clone()),
    }
}

pub(super) fn cpmm_is_base_in(
    pool: &RaydiumCpmmParams,
    input_mint: Pubkey,
    output_mint: Pubkey,
) -> Result<bool> {
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

pub(super) fn amm_v4_is_coin_in(
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

fn ensure_sol_hop_pair(
    sol_hop: &StonkFunSolHop,
    funding_mint: Pubkey,
    quote_mint: Pubkey,
) -> Result<()> {
    let funding_mint = normalize_native_sol(funding_mint);
    let quote = normalize_native_sol(quote_mint);
    match sol_hop {
        // Direction, pool pairs, amounts and every branch are validated by plan_route.
        StonkFunSolHop::Route(_) => Ok(()),
        StonkFunSolHop::RaydiumCpmm(pool) => {
            let a = normalize_native_sol(pool.base_mint);
            let b = normalize_native_sol(pool.quote_mint);
            if (a == funding_mint && b == quote) || (b == funding_mint && a == quote) {
                Ok(())
            } else {
                Err(anyhow!(
                    "Funding hop CPMM pool {}/{} does not match funding-asset/{}",
                    pool.base_mint,
                    pool.quote_mint,
                    quote
                ))
            }
        }
        StonkFunSolHop::RaydiumAmmV4(pool) => {
            let a = normalize_native_sol(pool.coin_mint);
            let b = normalize_native_sol(pool.pc_mint);
            if (a == funding_mint && b == quote) || (b == funding_mint && a == quote) {
                Ok(())
            } else {
                Err(anyhow!(
                    "Funding hop AMM v4 pool {}/{} does not match funding-asset/{}",
                    pool.coin_mint,
                    pool.pc_mint,
                    quote
                ))
            }
        }
    }
}

fn sol_hop_min_out(
    sol_hop: &StonkFunSolHop,
    amount_in: u64,
    input_mint: Pubkey,
    output_mint: Pubkey,
    slippage_basis_points: u64,
    is_buy: bool,
) -> Result<u64> {
    match sol_hop {
        StonkFunSolHop::Route(route) => Ok(super::stonkfun_quote_route::plan_route(
            route,
            input_mint,
            output_mint,
            amount_in,
            slippage_basis_points,
            is_buy,
        )?
        .minimum_credit),
        StonkFunSolHop::RaydiumCpmm(pool) => {
            let is_base_in = cpmm_is_base_in(pool, input_mint, output_mint)?;
            Ok(compute_swap_amount_for_pool(pool, is_base_in, amount_in, slippage_basis_points)?
                .min_amount_out)
        }
        StonkFunSolHop::RaydiumAmmV4(pool) => {
            let is_coin_in = amm_v4_is_coin_in(pool, input_mint, output_mint)?;
            Ok(compute_amm_v4_swap_amount(pool, is_coin_in, amount_in, slippage_basis_points)?
                .min_amount_out)
        }
    }
}

fn meme_leg_buy_min_out(
    meme_leg: &StonkFunMemeLeg,
    quote_amount_in: u64,
    meme_mint: Pubkey,
    slippage_basis_points: u64,
) -> Result<u64> {
    match meme_leg {
        StonkFunMemeLeg::Curve(params) => {
            Ok(get_buy_quote(quote_amount_in, params, 0, slippage_basis_points as u128)?
                .minimum_amount_out)
        }
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
    }
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
    // Endpoint programs must not leak into an intermediate mint's instruction.
    leg.input_token_program = (normalize_native_sol(input_mint)
        == normalize_native_sol(params.input_mint))
    .then_some(params.input_token_program)
    .flatten();
    leg.output_token_program = (normalize_native_sol(output_mint)
        == normalize_native_sol(params.output_mint))
    .then_some(params.output_token_program)
    .flatten();
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
    leg.input_token_program = (normalize_native_sol(input_mint)
        == normalize_native_sol(params.input_mint))
    .then_some(params.input_token_program)
    .flatten();
    leg.output_token_program = (normalize_native_sol(output_mint)
        == normalize_native_sol(params.output_mint))
    .then_some(params.output_token_program)
    .flatten();
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
    let meme_mint = params.output_mint;
    let quote_mint = meme_leg_quote_mint(&via.meme_leg, meme_mint)?;
    let funding_mint = normalize_native_sol(params.input_mint);
    if !is_native_sol(params.input_mint)
        && params.input_mint != crate::constants::USDC_TOKEN_ACCOUNT
        && funding_mint != quote_mint
    {
        return Err(anyhow!(
            "StonkFun routed buy expects SOL, WSOL or USDC input mint, got {}",
            params.input_mint
        ));
    }
    let input_amount = params
        .input_amount
        .filter(|&a| a > 0)
        .ok_or_else(|| anyhow!("StonkFun routed buy requires a non-zero input amount"))?;
    let slippage = params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE);
    let source_program = if funding_mint == quote_mint {
        meme_leg_quote_program(&via.meme_leg, quote_mint)
    } else {
        crate::constants::TOKEN_PROGRAM
    };
    if params.input_token_program.is_some_and(|p| p != source_program) {
        return Err(anyhow!("StonkFun input token program does not match the funding asset"));
    }
    let mut instructions = Vec::with_capacity(12);
    if params.input_mint == crate::constants::SOL_TOKEN_ACCOUNT && !params.create_input_mint_ata {
        instructions.extend(crate::trading::common::fund_existing_wsol(
            &params.payer.pubkey(),
            input_amount,
        ));
    }
    if params.create_input_mint_ata {
        if params.input_mint == crate::constants::SOL_TOKEN_ACCOUNT {
            instructions
                .extend(crate::trading::common::handle_wsol(&params.payer.pubkey(), input_amount));
        } else {
            super::token_account_setup::push_create_user_token_account(
                &mut instructions,
                &params.payer.pubkey(),
                &funding_mint,
                &source_program,
                params.open_seed_optimize,
            );
        }
    }

    // The wallet already uses the pool quote: build one meme-leg buy.
    if normalize_native_sol(quote_mint) == funding_mint {
        instructions.extend(
            build_leg_buy(
                params,
                meme_leg_as_dex_param(&via.meme_leg),
                funding_mint,
                meme_mint,
                input_amount,
                false,
                params.close_input_mint_ata,
                params.create_output_mint_ata,
                false,
                params.fixed_output_amount,
            )
            .await?,
        );
        return Ok(instructions);
    }

    ensure_sol_hop_pair(&via.sol_hop, funding_mint, quote_mint)?;

    // Match legs on hop1 min-out so hop2 cannot overspend the quote ATA.
    let quote_bridge =
        sol_hop_min_out(&via.sol_hop, input_amount, funding_mint, quote_mint, slippage, true)?;
    if quote_bridge == 0 {
        return Err(anyhow!("StonkFunViaSol SOL hop produced zero quote output"));
    }

    // Validate the meme leg can absorb that quote amount (also warms error paths).
    let _ = meme_leg_buy_min_out(&via.meme_leg, quote_bridge, meme_mint, slippage)?;

    // Persistent ATAs: WSOL / stock-quote accounts are expected to live across trades.
    // Only create them when the caller opts in (CreateMissing / Auto). HotPathMinimal
    // and AssumePrepared keep create/close flags false and skip ATA ix entirely.
    // Never close the intermediate quote ATA — leftover stock dust is intentional.
    let create_quote_ata = params.create_input_mint_ata || params.create_output_mint_ata;

    // Hop 1: funding asset → quote. Defer WSOL close until after both hops.
    let hop1 = build_leg_buy(
        params,
        sol_hop_as_dex_param(&via.sol_hop),
        funding_mint,
        quote_mint,
        input_amount,
        false,
        false,
        create_quote_ata,
        false,
        None,
    )
    .await?;
    instructions.extend(hop1);

    // Hop 2: quote → meme. Never create/close the quote ATA on this leg.
    let hop2 = build_leg_buy(
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
    instructions.extend(hop2);

    if params.close_input_mint_ata {
        crate::instruction::token_account_setup::push_close_wsol_if_needed(
            &mut instructions,
            &params.payer.pubkey(),
            &funding_mint,
        );
    }

    Ok(instructions)
}

async fn build_sell_via_sol(
    params: &SwapParams,
    via: &StonkFunViaSolParams,
) -> Result<Vec<Instruction>> {
    let meme_mint = params.input_mint;
    let quote_mint = meme_leg_quote_mint(&via.meme_leg, meme_mint)?;
    let funding_mint = normalize_native_sol(params.output_mint);
    if !is_native_sol(params.output_mint)
        && params.output_mint != crate::constants::USDC_TOKEN_ACCOUNT
        && funding_mint != quote_mint
    {
        return Err(anyhow!(
            "StonkFun routed sell expects SOL, WSOL or USDC output mint, got {}",
            params.output_mint
        ));
    }
    let meme_amount = params
        .input_amount
        .filter(|&a| a > 0)
        .ok_or_else(|| anyhow!("StonkFunViaSol sell requires a non-zero meme input amount"))?;
    let slippage = params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE);

    if normalize_native_sol(quote_mint) == funding_mint {
        return build_leg_sell(
            params,
            meme_leg_as_dex_param(&via.meme_leg),
            meme_mint,
            funding_mint,
            meme_amount,
            false,
            params.close_input_mint_ata,
            params.create_output_mint_ata,
            params.close_output_mint_ata,
            params.fixed_output_amount,
        )
        .await;
    }

    ensure_sol_hop_pair(&via.sol_hop, funding_mint, quote_mint)?;

    let quote_bridge = meme_leg_sell_min_out(&via.meme_leg, meme_amount, meme_mint, slippage)?;
    if quote_bridge == 0 {
        return Err(anyhow!("StonkFunViaSol meme leg produced zero quote output"));
    }
    let _ = sol_hop_min_out(&via.sol_hop, quote_bridge, quote_mint, funding_mint, slippage, false)?;

    // Same persistence policy as buy: do not force-create or close stock quote ATAs.
    let create_quote_ata = params.create_input_mint_ata || params.create_output_mint_ata;

    let mut instructions = Vec::with_capacity(12);

    // Hop 1: meme → quote. Never close the stock quote ATA after the sell hop.
    let hop1 = build_leg_sell(
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
    instructions.extend(hop1);

    // Hop 2: quote → funding asset. WSOL create/close follows the caller's output ATA flags.
    let hop2 = build_leg_sell(
        params,
        sol_hop_as_dex_param(&via.sol_hop),
        quote_mint,
        funding_mint,
        quote_bridge,
        false,
        false,
        params.create_output_mint_ata,
        params.close_output_mint_ata,
        params.fixed_output_amount,
    )
    .await?;
    instructions.extend(hop2);

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
            DexParamEnum::StonkFunQuoteRoute(route) => {
                super::stonkfun_quote_route::build_quote_route(params, route, true).await
            }
            DexParamEnum::RaydiumCpmm(_) => {
                RaydiumCpmmInstructionBuilder.build_buy_instructions(params).await
            }
            DexParamEnum::RaydiumAmmV4(_) => {
                RaydiumAmmV4InstructionBuilder.build_buy_instructions(params).await
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
            DexParamEnum::StonkFunQuoteRoute(route) => {
                super::stonkfun_quote_route::build_quote_route(params, route, false).await
            }
            DexParamEnum::RaydiumCpmm(_) => {
                RaydiumCpmmInstructionBuilder.build_sell_instructions(params).await
            }
            DexParamEnum::RaydiumAmmV4(_) => {
                RaydiumAmmV4InstructionBuilder.build_sell_instructions(params).await
            }
            _ => Err(anyhow!("Invalid protocol params for StonkFun")),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        common::GasFeeStrategy,
        instruction::utils::{
            bonk::accounts as launchlab_accounts,
            raydium_cpmm::{accounts as cpmm_accounts, SWAP_BASE_IN_DISCRIMINATOR},
        },
        swqos::TradeType,
        trading::core::params::{BonkParams, RaydiumAmmV4Params, RaydiumCpmmParams},
        utils::calc::common::calculate_min_amount_out,
    };
    use solana_sdk::{pubkey::Pubkey, signature::Keypair};
    use std::sync::Arc;

    fn pk(seed: u8) -> Pubkey {
        Pubkey::new_from_array([seed; 32])
    }

    pub(crate) fn curve_params(quote_mint: Pubkey) -> BonkParams {
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

    pub(crate) fn swap_params(
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
        }
    }

    #[tokio::test]
    async fn via_sol_curve_buy_composes_sol_hop_then_launchlab() {
        let stock = pk(40);
        let meme = pk(41);
        let via = StonkFunViaSolParams::curve(
            curve_params(stock),
            StonkFunSolHop::RaydiumCpmm(cpmm_pool(crate::constants::WSOL_TOKEN_ACCOUNT, stock)),
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
    async fn via_sol_graduated_sell_composes_meme_leg_then_sol_hop() {
        let stock = pk(50);
        let meme = pk(51);
        let via = StonkFunViaSolParams::graduated(
            cpmm_pool(stock, meme),
            StonkFunSolHop::RaydiumCpmm(cpmm_pool(crate::constants::WSOL_TOKEN_ACCOUNT, stock)),
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
            StonkFunSolHop::RaydiumCpmm(cpmm_pool(crate::constants::WSOL_TOKEN_ACCOUNT, other)),
        );
        let params = swap_params(
            TradeType::Buy,
            crate::constants::WSOL_TOKEN_ACCOUNT,
            meme,
            DexParamEnum::StonkFunViaSol(via),
        );
        let err = StonkFunInstructionBuilder.build_buy_instructions(&params).await.unwrap_err();
        assert!(err.to_string().contains("does not match funding-asset"));
    }

    #[tokio::test]
    async fn via_sol_hot_path_skips_ata_create_and_never_closes_quote() {
        let stock = pk(70);
        let meme = pk(71);
        let via = StonkFunViaSolParams::curve(
            curve_params(stock),
            StonkFunSolHop::RaydiumCpmm(cpmm_pool(crate::constants::WSOL_TOKEN_ACCOUNT, stock)),
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
        assert!(
            programs.contains(&crate::instruction::utils::raydium_amm_v4::accounts::RAYDIUM_AMM_V4)
        );
        assert!(programs.contains(&launchlab_accounts::BONK));
    }

    #[test]
    fn direct_funding_pair_validation_accepts_usdc_and_rejects_wrong_pool() {
        let quote = pk(60);
        let usdc = crate::constants::USDC_TOKEN_ACCOUNT;
        let hop = StonkFunSolHop::RaydiumCpmm(cpmm_pool(usdc, quote));
        assert!(ensure_sol_hop_pair(&hop, usdc, quote).is_ok());
        assert!(ensure_sol_hop_pair(&hop, crate::constants::WSOL_TOKEN_ACCOUNT, quote).is_err());
    }

    #[tokio::test]
    async fn inner_and_graduated_buy_sell_support_sol_wsol_usdc_without_extra_wsol_funding() {
        let stock = pk(60);
        let meme = pk(61);
        for graduated in [false, true] {
            for asset in [
                crate::constants::SOL_TOKEN_ACCOUNT,
                crate::constants::WSOL_TOKEN_ACCOUNT,
                crate::constants::USDC_TOKEN_ACCOUNT,
            ] {
                let hop = cpmm_pool(normalize_native_sol(asset), stock);
                let via = if graduated {
                    StonkFunViaSolParams::graduated(cpmm_pool(meme, stock), hop)
                } else {
                    StonkFunViaSolParams::curve(curve_params(stock), hop)
                };
                let mut buy = swap_params(TradeType::Buy, asset, meme, via.clone().into());
                buy.close_input_mint_ata = false;
                let buy_ixs =
                    StonkFunInstructionBuilder.build_buy_instructions(&buy).await.unwrap();
                let native_funding = buy_ixs
                    .iter()
                    .filter(|ix| {
                        ix.program_id == crate::constants::SYSTEM_PROGRAM
                            && ix.data.get(..4) == Some(&[2, 0, 0, 0])
                    })
                    .count();
                assert_eq!(
                    native_funding,
                    usize::from(asset == crate::constants::SOL_TOKEN_ACCOUNT)
                );
                assert_eq!(
                    buy_ixs
                        .iter()
                        .filter(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM)
                        .count(),
                    if graduated { 2 } else { 1 }
                );
                let mut sell = swap_params(TradeType::Sell, meme, asset, via.into());
                sell.close_input_mint_ata = false;
                sell.close_output_mint_ata = asset == crate::constants::SOL_TOKEN_ACCOUNT;
                let sell_ixs =
                    StonkFunInstructionBuilder.build_sell_instructions(&sell).await.unwrap();
                let closes = sell_ixs
                    .iter()
                    .filter(|ix| ix.program_id == crate::constants::TOKEN_PROGRAM && ix.data == [9])
                    .count();
                assert_eq!(closes, usize::from(asset == crate::constants::SOL_TOKEN_ACCOUNT));
            }
        }
    }

    #[tokio::test]
    async fn native_sol_still_funds_prepared_ata_while_existing_wsol_does_not() {
        let quote = pk(60);
        let meme = pk(61);
        let via = StonkFunViaSolParams::curve(
            curve_params(quote),
            cpmm_pool(crate::constants::WSOL_TOKEN_ACCOUNT, quote),
        );
        for asset in [crate::constants::SOL_TOKEN_ACCOUNT, crate::constants::WSOL_TOKEN_ACCOUNT] {
            let mut params = swap_params(TradeType::Buy, asset, meme, via.clone().into());
            params.create_input_mint_ata = false;
            params.create_output_mint_ata = false;
            params.close_input_mint_ata = false;
            let ixs = StonkFunInstructionBuilder.build_buy_instructions(&params).await.unwrap();
            assert_eq!(ixs.len(), if asset == crate::constants::SOL_TOKEN_ACCOUNT { 4 } else { 2 });
            assert!(!ixs
                .iter()
                .any(|ix| ix.program_id == crate::constants::ASSOCIATED_TOKEN_PROGRAM_ID));
        }
    }
    #[tokio::test]
    async fn direct_stock_quote_is_one_swap_for_both_venues_and_token_programs() {
        let stock = pk(60);
        let meme = pk(61);
        for graduated in [false, true] {
            for program in [crate::constants::TOKEN_PROGRAM, crate::constants::TOKEN_PROGRAM_2022] {
                let via = if graduated {
                    let mut pool = cpmm_pool(meme, stock);
                    pool.quote_token_program = program;
                    StonkFunViaSolParams::graduated_direct(pool)
                } else {
                    let mut curve = curve_params(stock);
                    curve.quote_token_program = program;
                    StonkFunViaSolParams::curve_direct(curve)
                };
                for is_buy in [true, false] {
                    let mut params = if is_buy {
                        swap_params(TradeType::Buy, stock, meme, via.clone().into())
                    } else {
                        swap_params(TradeType::Sell, meme, stock, via.clone().into())
                    };
                    params.close_input_mint_ata = false;
                    params.close_output_mint_ata = false;
                    if is_buy {
                        params.input_token_program = Some(program);
                    } else {
                        params.output_token_program = Some(program);
                    }
                    let ixs = if is_buy {
                        StonkFunInstructionBuilder.build_buy_instructions(&params).await.unwrap()
                    } else {
                        StonkFunInstructionBuilder.build_sell_instructions(&params).await.unwrap()
                    };
                    assert_eq!(
                        ixs.iter()
                            .filter(|ix| ix.program_id == cpmm_accounts::RAYDIUM_CPMM
                                || ix.program_id == launchlab_accounts::BONK)
                            .count(),
                        1
                    );
                    assert!(!ixs
                        .iter()
                        .any(|ix| ix.program_id == crate::constants::SYSTEM_PROGRAM
                            && ix.data.get(..4) == Some(&[2, 0, 0, 0])));
                }
            }
        }
    }

    #[test]
    fn calculate_min_amount_out_is_used_for_bridge_matching() {
        assert_eq!(calculate_min_amount_out(10_000, 100), 9_900);
    }
}
