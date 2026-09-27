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
        raydium_amm_v4::compute_swap_amount as compute_amm_v4_swap_amount,
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

fn meme_leg_quote_mint(meme_leg: &StonkFunMemeLeg, meme_mint: Pubkey) -> Result<Pubkey> {
    match meme_leg {
        StonkFunMemeLeg::Curve(params) => curve_quote_mint(params),
        StonkFunMemeLeg::Graduated(params) => graduated_quote_mint(params, meme_mint),
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

fn ensure_sol_hop_pair(sol_hop: &StonkFunSolHop, quote_mint: Pubkey) -> Result<()> {
    let wsol = crate::constants::WSOL_TOKEN_ACCOUNT;
    let quote = normalize_native_sol(quote_mint);
    match sol_hop {
        StonkFunSolHop::RaydiumCpmm(pool) => {
            let a = normalize_native_sol(pool.base_mint);
            let b = normalize_native_sol(pool.quote_mint);
            if (a == wsol && b == quote) || (b == wsol && a == quote) {
                Ok(())
            } else {
                Err(anyhow!(
                    "SOL hop CPMM pool {}/{} does not match WSOL/{}",
                    pool.base_mint,
                    pool.quote_mint,
                    quote
                ))
            }
        }
        StonkFunSolHop::RaydiumAmmV4(pool) => {
            let a = normalize_native_sol(pool.coin_mint);
            let b = normalize_native_sol(pool.pc_mint);
            if (a == wsol && b == quote) || (b == wsol && a == quote) {
                Ok(())
            } else {
                Err(anyhow!(
                    "SOL hop AMM v4 pool {}/{} does not match WSOL/{}",
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
) -> Result<u64> {
    match sol_hop {
        StonkFunSolHop::RaydiumCpmm(pool) => {
            let is_base_in = cpmm_is_base_in(pool, input_mint, output_mint)?;
            Ok(compute_swap_amount_for_pool(pool, is_base_in, amount_in, slippage_basis_points)?
                .min_amount_out)
        }
        StonkFunSolHop::RaydiumAmmV4(pool) => {
            let is_coin_in = amm_v4_is_coin_in(pool, input_mint, output_mint)?;
            Ok(compute_amm_v4_swap_amount(
                pool.coin_reserve,
                pool.pc_reserve,
                is_coin_in,
                amount_in,
                slippage_basis_points,
            )
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

    ensure_sol_hop_pair(&via.sol_hop, quote_mint)?;
    let (hop_params, hop_slippage) = hop_swap_params(params, via, slippage);

    // Match legs on hop1 min-out so hop2 cannot overspend the quote ATA.
    let quote_bridge =
        sol_hop_min_out(&via.sol_hop, sol_amount, wsol, quote_mint, hop_slippage)?;
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

    let mut instructions = Vec::with_capacity(12);

    // Hop 1: WSOL → quote. Defer WSOL close until after both hops.
    let hop1 = build_leg_buy(
        &hop_params,
        sol_hop_as_dex_param(&via.sol_hop),
        wsol,
        quote_mint,
        sol_amount,
        params.create_input_mint_ata,
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

    ensure_sol_hop_pair(&via.sol_hop, quote_mint)?;
    let (hop_params, hop_slippage) = hop_swap_params(params, via, slippage);

    let quote_bridge =
        meme_leg_sell_min_out(&via.meme_leg, meme_amount, meme_mint, slippage)?;
    if quote_bridge == 0 {
        return Err(anyhow!("StonkFunViaSol meme leg produced zero quote output"));
    }
    let _ = sol_hop_min_out(&via.sol_hop, quote_bridge, quote_mint, wsol, hop_slippage)?;

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

    // Hop 2: quote → WSOL. WSOL create/close follows the caller's output ATA flags.
    let hop2 = build_leg_sell(
        &hop_params,
        sol_hop_as_dex_param(&via.sol_hop),
        quote_mint,
        wsol,
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
mod tests {
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

    #[test]
    fn calculate_min_amount_out_is_used_for_bridge_matching() {
        assert_eq!(calculate_min_amount_out(10_000, 100), 9_900);
    }
}
