//! Pump.fun bonding-curve swap ix assembly ([`SwapParams`](crate::trading::core::params::SwapParams)).
//!
//! The SDK selects the legacy or V2 on-chain layout from `PumpFunParams.quote_mint`.
//! Native SOL-paired coins (`Pubkey::default()`, Solscan SOL sentinel, or WSOL sentinel)
//! keep the smaller legacy SOL layout by default. Non-native quote mints such as USDC use
//! the V2 27/26-account unified metas.

use crate::swqos::TradeType;
use crate::{
    common::bonding_curve::BondingCurveAccount,
    common::spl_token::close_account,
    constants::{trade::trade::DEFAULT_SLIPPAGE, TOKEN_PROGRAM_2022},
    trading::core::{
        params::{PumpFunParams, SwapParams},
        traits::InstructionBuilder,
    },
};
use crate::{
    instruction::{
        token_account_setup::{
            push_close_wsol_if_needed, push_create_or_wrap_user_token_account,
            push_create_user_token_account,
        },
        utils::pumpfun::{
            accounts, get_bonding_curve_pda, get_user_volume_accumulator_pda,
            global_constants::{self},
            pump_fun_fee_recipient_meta, resolve_creator_vault_for_ix_with_fee_sharing,
        },
    },
    utils::calc::{
        common::{calculate_with_slippage_buy, calculate_with_slippage_sell},
        pumpfun::{get_buy_token_amount_from_sol_amount, get_sell_sol_amount_from_token_amount},
    },
};
use anyhow::{anyhow, Result};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signer::Signer,
};

#[inline]
pub(super) fn effective_pump_mint_token_program(
    protocol_params: &PumpFunParams,
    mint: &Pubkey,
) -> Pubkey {
    let mut encoded = [0_u8; 44];
    let has_pump_suffix = bs58::encode(mint.as_ref())
        .onto(&mut encoded[..])
        .is_ok_and(|len| encoded[..len].ends_with(b"pump"));
    if has_pump_suffix {
        return TOKEN_PROGRAM_2022;
    }
    let tp = protocol_params.token_program;
    if tp == Pubkey::default() {
        TOKEN_PROGRAM_2022
    } else {
        tp
    }
}

/// Resolve quote mint and its token program from PumpFunParams.
/// `Pubkey::default()` / `SOL_TOKEN_ACCOUNT` / `WSOL_TOKEN_ACCOUNT` are native SOL-paired.
/// V2 helpers still need a concrete SPL mint, so native SOL resolves to WSOL internally.
#[inline]
fn effective_quote_mint_and_token_program(protocol_params: &PumpFunParams) -> (Pubkey, Pubkey) {
    let curve_quote_mint = protocol_params.bonding_curve.effective_quote_mint();
    let quote_mint = if protocol_params.quote_mint != Pubkey::default() {
        BondingCurveAccount::normalize_quote_mint(protocol_params.quote_mint)
    } else if curve_quote_mint != Pubkey::default() {
        curve_quote_mint
    } else {
        crate::constants::WSOL_TOKEN_ACCOUNT
    };
    (quote_mint, crate::constants::TOKEN_PROGRAM)
}

#[inline]
fn is_sol_quote_mint(quote_mint: &Pubkey) -> bool {
    *quote_mint == crate::constants::WSOL_TOKEN_ACCOUNT
        || *quote_mint == crate::constants::SOL_TOKEN_ACCOUNT
}

#[inline]
fn is_native_sol_settlement_mint(mint: &Pubkey) -> bool {
    *mint == crate::constants::SOL_TOKEN_ACCOUNT || *mint == Pubkey::default()
}

#[inline]
fn is_explicit_wsol_settlement_mint(mint: &Pubkey) -> bool {
    *mint == crate::constants::WSOL_TOKEN_ACCOUNT
}

#[inline]
fn validate_v2_buy_quote_mint(input_mint: &Pubkey, quote_mint: &Pubkey) -> Result<()> {
    if is_sol_quote_mint(quote_mint) {
        if *input_mint == crate::constants::SOL_TOKEN_ACCOUNT
            || *input_mint == crate::constants::WSOL_TOKEN_ACCOUNT
        {
            return Ok(());
        }
    } else if input_mint == quote_mint {
        return Ok(());
    }
    Err(anyhow!(
        "PumpFun V2 buy input_mint {} does not match quote_mint {}; USDC quote pools must be bought with USDC, not SOL",
        input_mint,
        quote_mint
    ))
}

#[inline]
fn validate_v2_sell_quote_mint(output_mint: &Pubkey, quote_mint: &Pubkey) -> Result<()> {
    if is_sol_quote_mint(quote_mint) {
        if *output_mint == crate::constants::SOL_TOKEN_ACCOUNT
            || *output_mint == crate::constants::WSOL_TOKEN_ACCOUNT
        {
            return Ok(());
        }
    } else if output_mint == quote_mint {
        return Ok(());
    }
    Err(anyhow!(
        "PumpFun V2 sell output_mint {} does not match quote_mint {}; USDC quote pools settle to USDC, not SOL",
        output_mint,
        quote_mint
    ))
}

pub struct PumpFunInstructionBuilder;

#[async_trait::async_trait]
impl InstructionBuilder for PumpFunInstructionBuilder {
    async fn build_buy_instructions(&self, params: &SwapParams) -> Result<Vec<Instruction>> {
        if let crate::trading::core::params::DexParamEnum::PumpFunVersioned {
            params: curve,
            version,
        } = &params.protocol_params
        {
            return build_versioned(params, curve, *version, true);
        }
        if let crate::trading::core::params::DexParamEnum::PumpFunV3(v3) = &params.protocol_params {
            return super::pumpfun_v3::build(params, v3, true);
        }
        if let crate::trading::core::params::DexParamEnum::PumpFunQuoteRoute(route) =
            &params.protocol_params
        {
            validate_curve_route_endpoint(route, params.output_mint, true)?;
            return super::stonkfun_quote_route::build_quote_route(params, route, true).await;
        }
        build_buy(params)
    }

    async fn build_sell_instructions(&self, params: &SwapParams) -> Result<Vec<Instruction>> {
        if let crate::trading::core::params::DexParamEnum::PumpFunVersioned {
            params: curve,
            version,
        } = &params.protocol_params
        {
            return build_versioned(params, curve, *version, false);
        }
        if let crate::trading::core::params::DexParamEnum::PumpFunV3(v3) = &params.protocol_params {
            return super::pumpfun_v3::build(params, v3, false);
        }
        if let crate::trading::core::params::DexParamEnum::PumpFunQuoteRoute(route) =
            &params.protocol_params
        {
            validate_curve_route_endpoint(route, params.input_mint, false)?;
            return super::stonkfun_quote_route::build_quote_route(params, route, false).await;
        }
        build_sell(params)
    }
}

fn build_versioned(
    params: &SwapParams,
    curve: &PumpFunParams,
    version: crate::common::PumpFunVersion,
    buy: bool,
) -> Result<Vec<Instruction>> {
    use crate::common::PumpFunVersion;
    match version {
        PumpFunVersion::Legacy => {
            anyhow::ensure!(
                curve.bonding_curve.effective_quote_mint() == crate::constants::WSOL_TOKEN_ACCOUNT,
                "Legacy Pump.fun requires a native SOL curve"
            );
            if buy {
                build_buy_legacy(params)
            } else {
                build_sell_legacy(params)
            }
        }
        PumpFunVersion::V2 => {
            if buy {
                build_buy_unified(params)
            } else {
                build_sell_unified(params)
            }
        }
        PumpFunVersion::V3 => {
            // Existing unversioned native-SOL requests retain their reserve-based
            // sizing. Arbitrary token quotes need explicit fee/program snapshots.
            anyhow::ensure!(curve.bonding_curve.effective_quote_mint() == crate::constants::WSOL_TOKEN_ACCOUNT,
                "V3 token-quote trades require PumpFunV3Params or PumpFunQuoteRoute with current mint fee/program snapshots");
            let amount = params.input_amount.unwrap_or(0);
            let slippage = params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE);
            let c = &curve.bonding_curve;
            let estimated = if buy {
                get_buy_token_amount_from_sol_amount(
                    c.virtual_token_reserves as u128,
                    c.virtual_sol_reserves as u128,
                    c.real_token_reserves as u128,
                    curve.effective_creator_for_trade(),
                    amount,
                )
            } else {
                get_sell_sol_amount_from_token_amount(
                    c.virtual_token_reserves as u128,
                    c.virtual_sol_reserves as u128,
                    curve.effective_creator_for_trade(),
                    amount,
                )
            };
            anyhow::ensure!(!buy || params.fixed_output_amount.is_some() || estimated < c.real_token_reserves,
                "V3 buy reaches graduation; supply PumpFunV3Params with an explicit current output minimum");
            let v3 = crate::PumpFunV3Params {
                curve: curve.clone(),
                quote_token_program: crate::constants::TOKEN_PROGRAM,
                minimum_output_amount: calculate_with_slippage_sell(estimated, slippage),
                base_transfer_fee: Default::default(),
                quote_transfer_fee: Default::default(),
            };
            let mut sized = params.clone();
            if buy
                && params.fixed_output_amount.is_none()
                && params.use_exact_sol_amount == Some(false)
            {
                sized.fixed_output_amount = Some(estimated);
                sized.input_amount = Some(calculate_with_slippage_buy(amount, slippage));
            }
            super::pumpfun_v3::build(&sized, &v3, buy)
        }
    }
}

fn validate_curve_route_endpoint(
    route: &crate::PumpFunQuoteRoute,
    mint: Pubkey,
    buy: bool,
) -> Result<()> {
    let endpoint = if buy { route.buy_hops.last() } else { route.sell_hops.first() };
    if !endpoint.is_some_and(|hop| {
        matches!(&hop.venue,
        crate::PumpFunQuoteVenue::PumpFun { mint: base, .. }
        | crate::PumpFunQuoteVenue::PumpFunV3 { mint: base, .. } if *base == mint)
    }) {
        return Err(anyhow!(
            "PumpFun quote route must buy into or sell from the requested bonding-curve mint"
        ));
    }
    Ok(())
}

/// Explicit exact-input hop limits; never infer a fill from a prior transaction.
pub(super) fn build_quote_hop(
    params: &SwapParams,
    minimum: u64,
    buy: bool,
    quote_program: Pubkey,
) -> Result<Vec<Instruction>> {
    if buy {
        build_buy_unified_with_minimum(params, Some(minimum), Some(quote_program))
    } else {
        let mut sell = params.clone();
        sell.fixed_output_amount = Some(minimum);
        build_sell_unified_with_quote_program(&sell, Some(quote_program))
    }
}

#[inline]
fn should_use_v2_layout(params: &SwapParams) -> Result<bool> {
    let protocol_params = params
        .protocol_params
        .as_any()
        .downcast_ref::<PumpFunParams>()
        .ok_or_else(|| anyhow!("Invalid protocol params for PumpFun"))?;
    let (quote_mint, _) = effective_quote_mint_and_token_program(protocol_params);
    if !is_sol_quote_mint(&quote_mint) {
        return Ok(true);
    }

    // Pump docs treat WSOL quote mint as the native SOL sentinel for SOL-paired curves.
    // Keep V1 for native SOL settlement; use V2 only when the caller explicitly wants to
    // spend/receive an existing WSOL ATA.
    Ok(match params.trade_type {
        TradeType::Buy | TradeType::CreateAndBuy => {
            is_explicit_wsol_settlement_mint(&params.input_mint)
        }
        TradeType::Sell => is_explicit_wsol_settlement_mint(&params.output_mint),
        TradeType::Create => false,
    })
}

#[inline]
fn build_buy(params: &SwapParams) -> Result<Vec<Instruction>> {
    if should_use_v2_layout(params)? {
        build_buy_unified(params)
    } else {
        build_buy_legacy(params)
    }
}

#[inline]
fn build_sell(params: &SwapParams) -> Result<Vec<Instruction>> {
    if should_use_v2_layout(params)? {
        build_sell_unified(params)
    } else {
        build_sell_legacy(params)
    }
}

// ---------------------------------------------------------------------------
// V1 (default) — 18 metas, no quote_mint ATA create
// ---------------------------------------------------------------------------

fn build_buy_legacy(params: &SwapParams) -> Result<Vec<Instruction>> {
    use crate::instruction::pumpfun_ix_data::{
        encode_pumpfun_buy_exact_quote_in_ix_data, encode_pumpfun_buy_ix_data, PumpFunIxVersion,
    };
    use crate::instruction::utils::pumpfun::{
        get_bonding_curve_v2_pda, get_protocol_extra_fee_recipient_random,
    };

    let protocol_params = params
        .protocol_params
        .as_any()
        .downcast_ref::<PumpFunParams>()
        .ok_or_else(|| anyhow!("Invalid protocol params for PumpFun"))?;

    if !is_native_sol_settlement_mint(&params.input_mint) {
        return Err(anyhow!(
            "PumpFun native SOL buy expects input_mint SOL; got {}. Use the matching non-native quote mint for V2 pools or WSOL input only when spending an existing WSOL ATA.",
            params.input_mint
        ));
    }

    let lamports_in = params.input_amount.unwrap_or(0);
    if lamports_in == 0 {
        return Err(anyhow!("Amount cannot be zero"));
    }

    let slippage_bp = params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE);

    let bonding_curve = &protocol_params.bonding_curve;
    let creator = protocol_params.effective_creator_for_trade();
    let creator_vault_account = resolve_creator_vault_for_ix_with_fee_sharing(
        &creator,
        protocol_params.creator_vault,
        &params.output_mint,
        protocol_params.fee_sharing_creator_vault_if_active,
    )
    .ok_or_else(|| anyhow!("creator_vault PDA derivation failed (creator={})", creator))?;

    let bonding_curve_addr = get_bonding_curve_pda(&params.output_mint).ok_or_else(|| {
        anyhow!("bonding_curve PDA derivation failed for mint {}", params.output_mint)
    })?;

    let is_mayhem_mode = bonding_curve.is_mayhem_mode;
    let token_program = effective_pump_mint_token_program(protocol_params, &params.output_mint);
    let token_program_meta = if token_program == TOKEN_PROGRAM_2022 {
        crate::constants::TOKEN_PROGRAM_2022_META
    } else {
        crate::constants::TOKEN_PROGRAM_META
    };

    let associated_bonding_curve =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &bonding_curve_addr,
            &params.output_mint,
            &token_program,
        );

    let user_token_account =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &params.output_mint,
            &token_program,
            params.open_seed_optimize,
        );

    let user_volume_accumulator = get_user_volume_accumulator_pda(&params.payer.pubkey())
        .ok_or_else(|| anyhow!("user_volume_accumulator PDA derivation failed"))?;

    let mut instructions = Vec::with_capacity(2);

    if params.create_output_mint_ata {
        instructions.extend(
            crate::common::fast_fn::create_associated_token_account_idempotent_fast_use_seed(
                &params.payer.pubkey(),
                &params.payer.pubkey(),
                &params.output_mint,
                &token_program,
                params.open_seed_optimize,
            ),
        );
    }

    // ── Legacy buy path ──
    let track_volume_val = if bonding_curve.is_cashback_coin { 1u8 } else { 0u8 };
    let ix_version = PumpFunIxVersion::Legacy { track_volume: track_volume_val };
    let buy_data = if let Some(token_amount) = params.fixed_output_amount {
        encode_pumpfun_buy_ix_data(token_amount, lamports_in, ix_version)
    } else {
        let buy_token_amount = get_buy_token_amount_from_sol_amount(
            bonding_curve.virtual_token_reserves as u128,
            bonding_curve.virtual_sol_reserves as u128,
            bonding_curve.real_token_reserves as u128,
            creator,
            lamports_in,
        );
        if params.use_exact_sol_amount.unwrap_or(true) {
            let min_tokens_out = calculate_with_slippage_sell(buy_token_amount, slippage_bp);
            encode_pumpfun_buy_exact_quote_in_ix_data(lamports_in, min_tokens_out, ix_version)
        } else {
            let max_sol_cost = calculate_with_slippage_buy(lamports_in, slippage_bp);
            encode_pumpfun_buy_ix_data(buy_token_amount, max_sol_cost, ix_version)
        }
    };

    let fee_recipient_meta =
        pump_fun_fee_recipient_meta(protocol_params.fee_recipient, is_mayhem_mode);

    let bonding_curve_v2 = get_bonding_curve_v2_pda(&params.output_mint).ok_or_else(|| {
        anyhow!("bonding_curve_v2 PDA derivation failed for mint {}", params.output_mint)
    })?;
    let mut metas: Vec<AccountMeta> = vec![
        global_constants::GLOBAL_ACCOUNT_META,
        fee_recipient_meta,
        AccountMeta::new_readonly(params.output_mint, false),
        AccountMeta::new(bonding_curve_addr, false),
        AccountMeta::new(associated_bonding_curve, false),
        AccountMeta::new(user_token_account, false),
        AccountMeta::new(params.payer.pubkey(), true),
        crate::constants::SYSTEM_PROGRAM_META,
        token_program_meta,
        AccountMeta::new(creator_vault_account, false),
        accounts::EVENT_AUTHORITY_META,
        accounts::PUMPFUN_META,
        accounts::GLOBAL_VOLUME_ACCUMULATOR_META,
        AccountMeta::new(user_volume_accumulator, false),
        accounts::FEE_CONFIG_META,
        accounts::FEE_PROGRAM_META,
    ];
    metas.push(AccountMeta::new_readonly(bonding_curve_v2, false));
    // Official @pump-fun/pump-sdk@2.0.0: buybackFeeRecipient remaining is writable.
    metas.push(AccountMeta::new(get_protocol_extra_fee_recipient_random(), false));

    instructions.push(Instruction::new_with_bytes(accounts::PUMPFUN, buy_data.as_slice(), metas));

    Ok(instructions)
}

fn build_sell_legacy(params: &SwapParams) -> Result<Vec<Instruction>> {
    use crate::instruction::pumpfun_ix_data::{encode_pumpfun_sell_ix_data, PumpFunIxVersion};
    use crate::instruction::utils::pumpfun::{
        get_bonding_curve_v2_pda, get_protocol_extra_fee_recipient_random,
        is_phantom_default_creator_vault,
    };

    let protocol_params = params
        .protocol_params
        .as_any()
        .downcast_ref::<PumpFunParams>()
        .ok_or_else(|| anyhow!("Invalid protocol params for PumpFun"))?;

    if !is_native_sol_settlement_mint(&params.output_mint) {
        return Err(anyhow!(
            "PumpFun native SOL sell expects output_mint SOL; got {}. Use the matching non-native quote mint for V2 pools or WSOL output only when receiving into an existing WSOL ATA.",
            params.output_mint
        ));
    }

    let token_amount = if let Some(amount) = params.input_amount {
        if amount == 0 {
            return Err(anyhow!("Amount cannot be zero"));
        }
        amount
    } else {
        return Err(anyhow!("Amount token is required"));
    };

    let slippage_bp = params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE);

    let bonding_curve = &protocol_params.bonding_curve;

    // 卖出时：优先信任观测到的 creator_vault（来自 gRPC / pump fee 事件）。
    // 不要通过 effective_creator 推导覆盖，避免 creator 被修改后推导出错 → Anchor 2006。
    let creator_vault_account = if protocol_params.creator_vault != Pubkey::default()
        && !is_phantom_default_creator_vault(&protocol_params.creator_vault)
    {
        protocol_params.creator_vault
    } else {
        resolve_creator_vault_for_ix_with_fee_sharing(
            &bonding_curve.creator,
            protocol_params.creator_vault,
            &params.input_mint,
            protocol_params.fee_sharing_creator_vault_if_active,
        )
        .ok_or_else(|| {
            anyhow!("creator_vault PDA derivation failed (curve_creator={})", bonding_curve.creator)
        })?
    };

    let min_sol_output = if let Some(fixed) = params.fixed_output_amount {
        fixed
    } else {
        let creator = protocol_params.effective_creator_for_trade();
        let sol_amount = get_sell_sol_amount_from_token_amount(
            bonding_curve.virtual_token_reserves as u128,
            bonding_curve.virtual_sol_reserves as u128,
            creator,
            token_amount,
        );
        calculate_with_slippage_sell(sol_amount, slippage_bp)
    };

    let bonding_curve_addr = get_bonding_curve_pda(&params.input_mint).ok_or_else(|| {
        anyhow!("bonding_curve PDA derivation failed for mint {}", params.input_mint)
    })?;

    let is_mayhem_mode = bonding_curve.is_mayhem_mode;
    let token_program = effective_pump_mint_token_program(protocol_params, &params.input_mint);
    let token_program_meta = if token_program == TOKEN_PROGRAM_2022 {
        crate::constants::TOKEN_PROGRAM_2022_META
    } else {
        crate::constants::TOKEN_PROGRAM_META
    };

    let associated_bonding_curve =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &bonding_curve_addr,
            &params.input_mint,
            &token_program,
        );

    let user_token_account =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &params.input_mint,
            &token_program,
            params.open_seed_optimize,
        );

    let user_volume_accumulator = get_user_volume_accumulator_pda(&params.payer.pubkey())
        .ok_or_else(|| anyhow!("user_volume_accumulator PDA derivation failed"))?;

    let mut instructions = Vec::with_capacity(2);
    let sell_data = encode_pumpfun_sell_ix_data(
        token_amount,
        min_sol_output,
        PumpFunIxVersion::Legacy { track_volume: 0 },
    );

    let fee_recipient_meta =
        pump_fun_fee_recipient_meta(protocol_params.fee_recipient, is_mayhem_mode);

    let bonding_curve_v2 = get_bonding_curve_v2_pda(&params.input_mint).ok_or_else(|| {
        anyhow!("bonding_curve_v2 PDA derivation failed for mint {}", params.input_mint)
    })?;
    let mut metas: Vec<AccountMeta> = vec![
        global_constants::GLOBAL_ACCOUNT_META,
        fee_recipient_meta,
        AccountMeta::new_readonly(params.input_mint, false),
        AccountMeta::new(bonding_curve_addr, false),
        AccountMeta::new(associated_bonding_curve, false),
        AccountMeta::new(user_token_account, false),
        AccountMeta::new(params.payer.pubkey(), true),
        crate::constants::SYSTEM_PROGRAM_META,
        AccountMeta::new(creator_vault_account, false),
        token_program_meta,
        accounts::EVENT_AUTHORITY_META,
        accounts::PUMPFUN_META,
        accounts::FEE_CONFIG_META,
        accounts::FEE_PROGRAM_META,
    ];

    if bonding_curve.is_cashback_coin {
        metas.push(AccountMeta::new(user_volume_accumulator, false));
    }

    metas.push(AccountMeta::new_readonly(bonding_curve_v2, false));
    // Official @pump-fun/pump-sdk@2.0.0: buybackFeeRecipient remaining is writable.
    metas.push(AccountMeta::new(get_protocol_extra_fee_recipient_random(), false));

    instructions.push(Instruction::new_with_bytes(accounts::PUMPFUN, sell_data.as_slice(), metas));

    if protocol_params.close_token_account_when_sell.unwrap_or(false) || params.close_input_mint_ata
    {
        instructions.push(close_account(
            &token_program,
            &user_token_account,
            &params.payer.pubkey(),
            &params.payer.pubkey(),
            &[&params.payer.pubkey()],
        )?);
    }

    Ok(instructions)
}

// ---------------------------------------------------------------------------
// V2 — 27 metas, quote_mint ATA create
// ---------------------------------------------------------------------------

fn build_buy_unified(params: &SwapParams) -> Result<Vec<Instruction>> {
    build_buy_unified_with_minimum(params, None, None)
}

fn build_buy_unified_with_minimum(
    params: &SwapParams,
    minimum: Option<u64>,
    quote_program: Option<Pubkey>,
) -> Result<Vec<Instruction>> {
    use crate::instruction::pumpfun_ix_data::{
        encode_pumpfun_buy_exact_quote_in_ix_data, encode_pumpfun_buy_ix_data, PumpFunIxVersion,
    };
    use crate::instruction::utils::pumpfun::{
        get_buyback_fee_recipient_random, get_fee_sharing_config_pda,
    };

    let protocol_params = params
        .protocol_params
        .as_any()
        .downcast_ref::<PumpFunParams>()
        .ok_or_else(|| anyhow!("Invalid protocol params for PumpFun"))?;

    let lamports_in = params.input_amount.unwrap_or(0);
    if lamports_in == 0 {
        return Err(anyhow!("Amount cannot be zero"));
    }

    let slippage_bp = params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE);

    let bonding_curve = &protocol_params.bonding_curve;
    let creator = protocol_params.effective_creator_for_trade();
    let creator_vault_account = resolve_creator_vault_for_ix_with_fee_sharing(
        &creator,
        protocol_params.creator_vault,
        &params.output_mint,
        protocol_params.fee_sharing_creator_vault_if_active,
    )
    .ok_or_else(|| anyhow!("creator_vault PDA derivation failed (creator={})", creator))?;

    let bonding_curve_addr = get_bonding_curve_pda(&params.output_mint).ok_or_else(|| {
        anyhow!("bonding_curve PDA derivation failed for mint {}", params.output_mint)
    })?;

    let is_mayhem_mode = bonding_curve.is_mayhem_mode;
    let base_token_program =
        effective_pump_mint_token_program(protocol_params, &params.output_mint);
    let base_token_program_meta = if base_token_program == TOKEN_PROGRAM_2022 {
        crate::constants::TOKEN_PROGRAM_2022_META
    } else {
        crate::constants::TOKEN_PROGRAM_META
    };

    let (quote_mint, quote_token_program) = effective_quote_mint_and_token_program(protocol_params);
    let quote_token_program = quote_program.unwrap_or(quote_token_program);
    validate_v2_buy_quote_mint(&params.input_mint, &quote_mint)?;
    let quote_token_program_meta = AccountMeta::new_readonly(quote_token_program, false);

    let associated_base_bonding_curve =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &bonding_curve_addr,
            &params.output_mint,
            &base_token_program,
        );

    let associated_base_user =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &params.output_mint,
            &base_token_program,
            params.open_seed_optimize,
        );

    let fee_recipient_meta =
        pump_fun_fee_recipient_meta(protocol_params.fee_recipient, is_mayhem_mode);
    let fee_recipient_pk = fee_recipient_meta.pubkey;
    let buyback_fee_recipient = get_buyback_fee_recipient_random();

    let associated_quote_fee_recipient =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &fee_recipient_pk,
            &quote_mint,
            &quote_token_program,
        );
    let associated_quote_buyback_fee_recipient =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &buyback_fee_recipient,
            &quote_mint,
            &quote_token_program,
        );
    let associated_quote_bonding_curve =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &bonding_curve_addr,
            &quote_mint,
            &quote_token_program,
        );
    let associated_quote_user =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &quote_mint,
            &quote_token_program,
            params.open_seed_optimize,
        );
    let associated_creator_vault =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &creator_vault_account,
            &quote_mint,
            &quote_token_program,
        );

    let sharing_config = get_fee_sharing_config_pda(&params.output_mint).ok_or_else(|| {
        anyhow!("sharing_config PDA derivation failed for mint {}", params.output_mint)
    })?;

    let user_volume_accumulator = get_user_volume_accumulator_pda(&params.payer.pubkey())
        .ok_or_else(|| anyhow!("user_volume_accumulator PDA derivation failed"))?;
    let associated_user_volume_accumulator =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &user_volume_accumulator,
            &quote_mint,
            &quote_token_program,
        );

    let mut instructions = Vec::with_capacity(6);

    if params.create_output_mint_ata {
        instructions.extend(
            crate::common::fast_fn::create_associated_token_account_idempotent_fast_use_seed(
                &params.payer.pubkey(),
                &params.payer.pubkey(),
                &params.output_mint,
                &base_token_program,
                params.open_seed_optimize,
            ),
        );
    }

    let (buy_data, quote_amount_to_fund) = if let Some(minimum) = minimum {
        (
            encode_pumpfun_buy_exact_quote_in_ix_data(lamports_in, minimum, PumpFunIxVersion::V2),
            lamports_in,
        )
    } else if let Some(token_amount) = params.fixed_output_amount {
        (encode_pumpfun_buy_ix_data(token_amount, lamports_in, PumpFunIxVersion::V2), lamports_in)
    } else {
        let buy_token_amount = get_buy_token_amount_from_sol_amount(
            bonding_curve.virtual_token_reserves as u128,
            bonding_curve.virtual_sol_reserves as u128,
            bonding_curve.real_token_reserves as u128,
            creator,
            lamports_in,
        );
        if params.use_exact_sol_amount.unwrap_or(true) {
            let min_tokens_out = calculate_with_slippage_sell(buy_token_amount, slippage_bp);
            (
                encode_pumpfun_buy_exact_quote_in_ix_data(
                    lamports_in,
                    min_tokens_out,
                    PumpFunIxVersion::V2,
                ),
                lamports_in,
            )
        } else {
            let max_sol_cost = calculate_with_slippage_buy(lamports_in, slippage_bp);
            (
                encode_pumpfun_buy_ix_data(buy_token_amount, max_sol_cost, PumpFunIxVersion::V2),
                max_sol_cost,
            )
        }
    };

    let should_use_native_sol_for_wsol_quote = quote_mint == crate::constants::WSOL_TOKEN_ACCOUNT
        && params.input_mint == crate::constants::SOL_TOKEN_ACCOUNT;
    if params.create_input_mint_ata && !should_use_native_sol_for_wsol_quote {
        push_create_or_wrap_user_token_account(
            &mut instructions,
            &params.payer.pubkey(),
            &quote_mint,
            &quote_token_program,
            quote_amount_to_fund,
            params.open_seed_optimize,
        );
    }

    let metas: Vec<AccountMeta> = vec![
        global_constants::GLOBAL_ACCOUNT_META,
        AccountMeta::new_readonly(params.output_mint, false),
        AccountMeta::new_readonly(quote_mint, false),
        base_token_program_meta,
        quote_token_program_meta,
        AccountMeta::new_readonly(accounts::ASSOCIATED_TOKEN_PROGRAM, false),
        fee_recipient_meta,
        AccountMeta::new(associated_quote_fee_recipient, false),
        AccountMeta::new(buyback_fee_recipient, false),
        AccountMeta::new(associated_quote_buyback_fee_recipient, false),
        AccountMeta::new(bonding_curve_addr, false),
        AccountMeta::new(associated_base_bonding_curve, false),
        AccountMeta::new(associated_quote_bonding_curve, false),
        AccountMeta::new(params.payer.pubkey(), true),
        AccountMeta::new(associated_base_user, false),
        AccountMeta::new(associated_quote_user, false),
        AccountMeta::new(creator_vault_account, false),
        AccountMeta::new(associated_creator_vault, false),
        AccountMeta::new_readonly(sharing_config, false),
        accounts::GLOBAL_VOLUME_ACCUMULATOR_META,
        AccountMeta::new(user_volume_accumulator, false),
        AccountMeta::new(associated_user_volume_accumulator, false),
        accounts::FEE_CONFIG_META,
        accounts::FEE_PROGRAM_META,
        crate::constants::SYSTEM_PROGRAM_META,
        accounts::EVENT_AUTHORITY_META,
        accounts::PUMPFUN_META,
    ];

    instructions.push(Instruction::new_with_bytes(accounts::PUMPFUN, buy_data.as_slice(), metas));

    if params.close_input_mint_ata && !should_use_native_sol_for_wsol_quote {
        push_close_wsol_if_needed(&mut instructions, &params.payer.pubkey(), &quote_mint);
    }

    Ok(instructions)
}

fn build_sell_unified(params: &SwapParams) -> Result<Vec<Instruction>> {
    build_sell_unified_with_quote_program(params, None)
}

fn build_sell_unified_with_quote_program(
    params: &SwapParams,
    quote_program: Option<Pubkey>,
) -> Result<Vec<Instruction>> {
    use crate::instruction::pumpfun_ix_data::{encode_pumpfun_sell_ix_data, PumpFunIxVersion};
    use crate::instruction::utils::pumpfun::{
        get_buyback_fee_recipient_random, get_fee_sharing_config_pda,
        is_phantom_default_creator_vault,
    };

    let protocol_params = params
        .protocol_params
        .as_any()
        .downcast_ref::<PumpFunParams>()
        .ok_or_else(|| anyhow!("Invalid protocol params for PumpFun"))?;

    let token_amount = if let Some(amount) = params.input_amount {
        if amount == 0 {
            return Err(anyhow!("Amount cannot be zero"));
        }
        amount
    } else {
        return Err(anyhow!("Amount token is required"));
    };

    let slippage_bp = params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE);

    let bonding_curve = &protocol_params.bonding_curve;

    // 卖出时：优先信任观测到的 creator_vault（来自 gRPC / pump fee 事件）。
    // 不要通过 effective_creator 推导覆盖，避免 creator 被修改后推导出错 → Anchor 2006。
    let creator_vault_account = if protocol_params.creator_vault != Pubkey::default()
        && !is_phantom_default_creator_vault(&protocol_params.creator_vault)
    {
        protocol_params.creator_vault
    } else {
        resolve_creator_vault_for_ix_with_fee_sharing(
            &bonding_curve.creator,
            protocol_params.creator_vault,
            &params.input_mint,
            protocol_params.fee_sharing_creator_vault_if_active,
        )
        .ok_or_else(|| {
            anyhow!("creator_vault PDA derivation failed (curve_creator={})", bonding_curve.creator)
        })?
    };

    let min_sol_output = if let Some(fixed) = params.fixed_output_amount {
        fixed
    } else {
        let creator = protocol_params.effective_creator_for_trade();
        let sol_amount = get_sell_sol_amount_from_token_amount(
            bonding_curve.virtual_token_reserves as u128,
            bonding_curve.virtual_sol_reserves as u128,
            creator,
            token_amount,
        );
        calculate_with_slippage_sell(sol_amount, slippage_bp)
    };

    let bonding_curve_addr = get_bonding_curve_pda(&params.input_mint).ok_or_else(|| {
        anyhow!("bonding_curve PDA derivation failed for mint {}", params.input_mint)
    })?;

    let is_mayhem_mode = bonding_curve.is_mayhem_mode;
    let base_token_program = effective_pump_mint_token_program(protocol_params, &params.input_mint);
    let base_token_program_meta = if base_token_program == TOKEN_PROGRAM_2022 {
        crate::constants::TOKEN_PROGRAM_2022_META
    } else {
        crate::constants::TOKEN_PROGRAM_META
    };

    let (quote_mint, quote_token_program) = effective_quote_mint_and_token_program(protocol_params);
    let quote_token_program = quote_program.unwrap_or(quote_token_program);
    validate_v2_sell_quote_mint(&params.output_mint, &quote_mint)?;
    let quote_token_program_meta = AccountMeta::new_readonly(quote_token_program, false);

    let associated_base_bonding_curve =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &bonding_curve_addr,
            &params.input_mint,
            &base_token_program,
        );

    let associated_base_user =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &params.input_mint,
            &base_token_program,
            params.open_seed_optimize,
        );

    let fee_recipient_meta =
        pump_fun_fee_recipient_meta(protocol_params.fee_recipient, is_mayhem_mode);
    let fee_recipient_pk = fee_recipient_meta.pubkey;
    let buyback_fee_recipient = get_buyback_fee_recipient_random();

    let associated_quote_fee_recipient =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &fee_recipient_pk,
            &quote_mint,
            &quote_token_program,
        );
    let associated_quote_buyback_fee_recipient =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &buyback_fee_recipient,
            &quote_mint,
            &quote_token_program,
        );
    let associated_quote_bonding_curve =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &bonding_curve_addr,
            &quote_mint,
            &quote_token_program,
        );
    let associated_quote_user =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &quote_mint,
            &quote_token_program,
            params.open_seed_optimize,
        );
    let associated_creator_vault =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &creator_vault_account,
            &quote_mint,
            &quote_token_program,
        );

    let sharing_config = get_fee_sharing_config_pda(&params.input_mint).ok_or_else(|| {
        anyhow!("sharing_config PDA derivation failed for mint {}", params.input_mint)
    })?;

    let user_volume_accumulator = get_user_volume_accumulator_pda(&params.payer.pubkey())
        .ok_or_else(|| anyhow!("user_volume_accumulator PDA derivation failed"))?;
    let associated_user_volume_accumulator =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &user_volume_accumulator,
            &quote_mint,
            &quote_token_program,
        );

    let mut instructions = Vec::with_capacity(4);

    if params.create_output_mint_ata {
        push_create_user_token_account(
            &mut instructions,
            &params.payer.pubkey(),
            &quote_mint,
            &quote_token_program,
            params.open_seed_optimize,
        );
    }

    let sell_data = encode_pumpfun_sell_ix_data(token_amount, min_sol_output, PumpFunIxVersion::V2);

    let metas: Vec<AccountMeta> = vec![
        global_constants::GLOBAL_ACCOUNT_META,
        AccountMeta::new_readonly(params.input_mint, false),
        AccountMeta::new_readonly(quote_mint, false),
        base_token_program_meta,
        quote_token_program_meta,
        AccountMeta::new_readonly(accounts::ASSOCIATED_TOKEN_PROGRAM, false),
        fee_recipient_meta,
        AccountMeta::new(associated_quote_fee_recipient, false),
        AccountMeta::new(buyback_fee_recipient, false),
        AccountMeta::new(associated_quote_buyback_fee_recipient, false),
        AccountMeta::new(bonding_curve_addr, false),
        AccountMeta::new(associated_base_bonding_curve, false),
        AccountMeta::new(associated_quote_bonding_curve, false),
        AccountMeta::new(params.payer.pubkey(), true),
        AccountMeta::new(associated_base_user, false),
        AccountMeta::new(associated_quote_user, false),
        AccountMeta::new(creator_vault_account, false),
        AccountMeta::new(associated_creator_vault, false),
        AccountMeta::new_readonly(sharing_config, false),
        AccountMeta::new(user_volume_accumulator, false),
        AccountMeta::new(associated_user_volume_accumulator, false),
        accounts::FEE_CONFIG_META,
        accounts::FEE_PROGRAM_META,
        crate::constants::SYSTEM_PROGRAM_META,
        accounts::EVENT_AUTHORITY_META,
        accounts::PUMPFUN_META,
    ];

    instructions.push(Instruction::new_with_bytes(accounts::PUMPFUN, sell_data.as_slice(), metas));

    if protocol_params.close_token_account_when_sell.unwrap_or(false) || params.close_input_mint_ata
    {
        instructions.push(close_account(
            &base_token_program,
            &associated_base_user,
            &params.payer.pubkey(),
            &params.payer.pubkey(),
            &[&params.payer.pubkey()],
        )?);
    }

    if params.close_output_mint_ata {
        push_close_wsol_if_needed(&mut instructions, &params.payer.pubkey(), &quote_mint);
    }

    Ok(instructions)
}

// ---------------------------------------------------------------------------
// Shared: claim_cashback (independent of V1/V2)
// ---------------------------------------------------------------------------

/// Claim cashback (UserVolumeAccumulator → user lamports).
pub fn claim_cashback_pumpfun_instruction(payer: &Pubkey) -> Option<Instruction> {
    const CLAIM_CASHBACK_DISCRIMINATOR: [u8; 8] = [37, 58, 35, 126, 190, 53, 228, 197];
    let user_volume_accumulator = get_user_volume_accumulator_pda(payer)?;
    let ix_accounts = vec![
        AccountMeta::new(*payer, true),
        AccountMeta::new(user_volume_accumulator, false),
        crate::constants::SYSTEM_PROGRAM_META,
        accounts::EVENT_AUTHORITY_META,
        accounts::PUMPFUN_META,
    ];
    let ix =
        Instruction::new_with_bytes(accounts::PUMPFUN, &CLAIM_CASHBACK_DISCRIMINATOR, ix_accounts);
    Some(ix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        common::{bonding_curve::BondingCurveAccount, GasFeeStrategy},
        constants::TOKEN_PROGRAM,
        trading::core::params::{DexParamEnum, PumpFunParams, SwapParams},
    };
    use solana_sdk::signature::Keypair;
    use std::sync::Arc;

    fn pump_mint() -> Pubkey {
        "E3JvmGcGFDzhu2Cnxyeq5BRvN7HH9JZUsfAUh2v8pump".parse().unwrap()
    }

    fn swap_params_for_buy(mint: Pubkey, token_program: Pubkey) -> SwapParams {
        let bonding_curve =
            crate::instruction::utils::pumpfun::get_bonding_curve_pda(&mint).unwrap();
        let creator = Pubkey::new_unique();
        let creator_vault =
            crate::instruction::utils::pumpfun::get_creator_vault_pda(&creator).unwrap();
        let bc = BondingCurveAccount {
            account: bonding_curve,
            virtual_token_reserves: global_constants::INITIAL_VIRTUAL_TOKEN_RESERVES,
            virtual_sol_reserves: global_constants::INITIAL_VIRTUAL_SOL_RESERVES,
            real_token_reserves: global_constants::INITIAL_REAL_TOKEN_RESERVES,
            creator,
            ..Default::default()
        };
        let params = PumpFunParams {
            bonding_curve: Arc::new(bc),
            associated_bonding_curve: Pubkey::default(),
            observed_trade_creator: Some(creator),
            creator_vault,
            fee_sharing_creator_vault_if_active: None,
            token_program,
            close_token_account_when_sell: None,
            fee_recipient: global_constants::FEE_RECIPIENT,
            quote_mint: Pubkey::default(),
        };

        SwapParams {
            rpc: None,
            payer: Arc::new(Keypair::new()),
            trade_type: crate::swqos::TradeType::Buy,
            input_mint: crate::constants::SOL_TOKEN_ACCOUNT,
            input_token_program: None,
            output_mint: mint,
            output_token_program: None,
            input_amount: Some(10_000_000),
            slippage_basis_points: Some(300),
            address_lookup_table_accounts: Vec::new(),
            recent_blockhash: None,
            wait_tx_confirmed: false,
            protocol_params: DexParamEnum::PumpFun(params),
            open_seed_optimize: true,
            swqos_clients: Arc::new(Vec::new()),
            middleware_manager: None,
            durable_nonce: None,
            with_tip: true,
            create_input_mint_ata: false,
            close_input_mint_ata: false,
            create_output_mint_ata: true,
            close_output_mint_ata: false,
            fixed_output_amount: None,
            gas_fee_strategy: GasFeeStrategy::new(),
            simulate: false,
            log_enabled: false,
            wait_for_all_submits: false,
            use_dedicated_sender_threads: false,
            sender_thread_cores: None,
            max_sender_concurrency: 0,
            effective_core_ids: Arc::new(Vec::new()),
            check_min_tip: false,
            transaction_version: crate::common::TradeTransactionVersion::V0,
            grpc_recv_us: None,
            use_exact_sol_amount: Some(true),
        }
    }

    #[test]
    fn test_claim_cashback_instruction() {
        let payer = Pubkey::new_unique();
        let ix = claim_cashback_pumpfun_instruction(&payer).unwrap();
        let accumulator = get_user_volume_accumulator_pda(&payer).unwrap();

        assert_eq!(ix.program_id, accounts::PUMPFUN);
        assert_eq!(ix.data, [37, 58, 35, 126, 190, 53, 228, 197]);
        assert_eq!(
            ix.accounts,
            vec![
                AccountMeta::new(payer, true),
                AccountMeta::new(accumulator, false),
                crate::constants::SYSTEM_PROGRAM_META,
                accounts::EVENT_AUTHORITY_META,
                accounts::PUMPFUN_META,
            ]
        );
    }

    #[tokio::test]
    async fn official_curve_route_preserves_final_limit_and_native_sol_settlement() {
        use crate::constants::{SOL_TOKEN_ACCOUNT, USDC_TOKEN_ACCOUNT, WSOL_TOKEN_ACCOUNT};
        use crate::{PumpFunQuoteHop, PumpFunQuoteRoute, PumpFunQuoteVenue, PumpRouteOptions};
        for asset in [SOL_TOKEN_ACCOUNT, USDC_TOKEN_ACCOUNT] {
            for buy in [true, false] {
                let middle = Pubkey::new_unique();
                let target = Pubkey::new_unique();
                let mut params = swap_params_for_buy(target, TOKEN_PROGRAM_2022);
                let venue = |mint, quote| {
                    let DexParamEnum::PumpFun(mut curve) =
                        swap_params_for_buy(mint, TOKEN_PROGRAM_2022).protocol_params
                    else {
                        unreachable!()
                    };
                    curve.quote_mint = quote;
                    Arc::make_mut(&mut curve.bonding_curve).quote_mint = quote;
                    curve.associated_bonding_curve =
                        crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                            &curve.bonding_curve.account,
                            &mint,
                            &TOKEN_PROGRAM_2022,
                        );
                    PumpFunQuoteVenue::PumpFun {
                        mint,
                        curve,
                        quote_token_program: TOKEN_PROGRAM_2022,
                        base_transfer_fee: Default::default(),
                        quote_transfer_fee: Default::default(),
                    }
                };
                let currency = if asset == SOL_TOKEN_ACCOUNT { WSOL_TOKEN_ACCOUNT } else { asset };
                let mut steps = vec![
                    PumpFunQuoteHop::exact_in(venue(middle, currency), asset, middle, 900),
                    PumpFunQuoteHop::exact_in(venue(target, middle), middle, target, 800),
                ];
                if asset == SOL_TOKEN_ACCOUNT {
                    if let PumpFunQuoteVenue::PumpFun { quote_token_program, .. } =
                        &mut steps[0].venue
                    {
                        *quote_token_program = crate::constants::TOKEN_PROGRAM;
                    }
                }
                if !buy {
                    steps.reverse();
                    for step in &mut steps {
                        std::mem::swap(&mut step.input_mint, &mut step.output_mint);
                    }
                }
                let options = PumpRouteOptions {
                    minimum_output_amount: Some(700),
                    buyback_recipient: Some(params.payer.pubkey()),
                    ..Default::default()
                };
                let route = if buy {
                    PumpFunQuoteRoute::buy(steps).with_buy_execution(options)
                } else {
                    PumpFunQuoteRoute::sell(steps).with_sell_execution(options)
                };
                params.input_mint = if buy { asset } else { target };
                params.output_mint = if buy { target } else { asset };
                params.protocol_params = DexParamEnum::PumpFunQuoteRoute(route);
                params.create_input_mint_ata = true;
                params.close_output_mint_ata = !buy;
                let ixs = if buy {
                    PumpFunInstructionBuilder.build_buy_instructions(&params).await
                } else {
                    PumpFunInstructionBuilder.build_sell_instructions(&params).await
                }
                .unwrap();
                let swaps: Vec<_> = ixs
                    .iter()
                    .filter(|ix| ix.data.starts_with(&[43, 100, 73, 19, 233, 246, 111, 148]))
                    .collect();
                assert_eq!(swaps.len(), 1);
                assert_eq!(&swaps[0].data[8..16], &params.input_amount.unwrap().to_le_bytes());
                assert_eq!(&swaps[0].data[16..24], &700u64.to_le_bytes());
                assert_eq!(swaps[0].accounts.len(), 26);
                assert!(
                    !ixs.iter().any(|ix| ix.program_id == Pubkey::default()),
                    "curve currency uses wallet SOL without wrapping"
                );
                assert!(
                    !ixs.iter().any(|ix| ix.data == [9]),
                    "do not close the unused native curve WSOL ATA"
                );
            }
        }
    }

    #[tokio::test]
    async fn stock_curve_route_preserves_amounts_programs_and_account_lifecycle() {
        use crate::trading::core::params::{RaydiumCpmmParams, TokenTransferFee};
        use crate::{PumpFunQuoteHop, PumpFunQuoteRoute, PumpFunQuoteVenue};
        for asset in [
            crate::constants::SOL_TOKEN_ACCOUNT,
            crate::constants::WSOL_TOKEN_ACCOUNT,
            crate::constants::USDC_TOKEN_ACCOUNT,
        ] {
            for buy in [true, false] {
                let mint = Pubkey::new_unique();
                let stock = Pubkey::new_unique();
                let mut params = swap_params_for_buy(mint, TOKEN_PROGRAM_2022);
                let DexParamEnum::PumpFun(mut curve) = params.protocol_params.clone() else {
                    unreachable!()
                };
                curve.quote_mint = stock;
                Arc::make_mut(&mut curve.bonding_curve).quote_mint = stock;
                curve.close_token_account_when_sell = Some(true); // Must not close a route leg's ATA.
                let venue = PumpFunQuoteVenue::PumpFun {
                    mint,
                    curve,
                    quote_token_program: TOKEN_PROGRAM_2022,
                    base_transfer_fee: TokenTransferFee::default(),
                    quote_transfer_fee: TokenTransferFee::default(),
                };
                let funding = if asset == crate::constants::SOL_TOKEN_ACCOUNT {
                    crate::constants::WSOL_TOKEN_ACCOUNT
                } else {
                    asset
                };
                let conversion = RaydiumCpmmParams::from_trade(
                    Pubkey::new_unique(),
                    Pubkey::new_unique(),
                    funding,
                    stock,
                    Pubkey::new_unique(),
                    Pubkey::new_unique(),
                    TOKEN_PROGRAM,
                    TOKEN_PROGRAM_2022,
                    Pubkey::new_unique(),
                    1_000_000_000,
                    1_000_000_000,
                );
                let bridge = PumpFunQuoteVenue::RaydiumCpmm(conversion);
                let route = if buy {
                    PumpFunQuoteRoute::buy(vec![
                        PumpFunQuoteHop::exact_in(bridge, asset, stock, 900),
                        PumpFunQuoteHop::exact_in(venue, stock, mint, 800),
                    ])
                } else {
                    PumpFunQuoteRoute::sell(vec![
                        PumpFunQuoteHop::exact_in(venue, mint, stock, 900),
                        PumpFunQuoteHop::exact_in(bridge, stock, asset, 800),
                    ])
                };
                params.protocol_params = DexParamEnum::PumpFunQuoteRoute(route.clone());
                params.input_mint = if buy { asset } else { mint };
                params.output_mint = if buy { mint } else { asset };
                params.input_amount = Some(1000);
                params.trade_type = if buy { TradeType::Buy } else { TradeType::Sell };
                params.create_input_mint_ata = true;
                params.create_output_mint_ata = true;
                params.close_output_mint_ata = !buy && asset == crate::constants::SOL_TOKEN_ACCOUNT;
                let ixs = if buy {
                    PumpFunInstructionBuilder.build_buy_instructions(&params).await
                } else {
                    PumpFunInstructionBuilder.build_sell_instructions(&params).await
                }
                .unwrap();
                let curve_index =
                    ixs.iter().position(|ix| ix.program_id == accounts::PUMPFUN).unwrap();
                let bridge_index = ixs
                    .iter()
                    .position(|ix| {
                        ix.program_id
                            == crate::instruction::utils::raydium_cpmm::accounts::RAYDIUM_CPMM
                    })
                    .unwrap();
                assert_eq!(curve_index > bridge_index, buy);
                assert_eq!(
                    u64::from_le_bytes(ixs[bridge_index].data[16..24].try_into().unwrap()),
                    if buy { 900 } else { 800 }
                );
                let ix = &ixs[curve_index];
                assert_eq!(
                    &ix.data[..8],
                    if buy {
                        &crate::instruction::utils::pumpfun::BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR
                    } else {
                        &crate::instruction::utils::pumpfun::SELL_V2_DISCRIMINATOR
                    }
                );
                assert_eq!(
                    u64::from_le_bytes(ix.data[8..16].try_into().unwrap()),
                    if buy { 900 } else { 1000 }
                );
                assert_eq!(
                    u64::from_le_bytes(ix.data[16..24].try_into().unwrap()),
                    if buy { 800 } else { 900 }
                );
                assert_eq!(ix.accounts[2].pubkey, stock);
                assert_eq!(ix.accounts[4].pubkey, TOKEN_PROGRAM_2022);
                let stock_ata = crate::common::spl_associated_token_account::get_associated_token_address_with_program_id(&params.payer.pubkey(), &stock, &TOKEN_PROGRAM_2022);
                assert_eq!(ix.accounts[15].pubkey, stock_ata);
                assert!(!ixs
                    .iter()
                    .any(|ix| ix.program_id == TOKEN_PROGRAM_2022 && ix.data == [9]));
                let preview =
                    route.preview(params.input_mint, params.output_mint, 1000, 100, buy).unwrap();
                assert_eq!(preview.hops[1].input_amount, 900);
                assert_eq!(preview.minimum_output_credit, 800);
                // The same guaranteed-balance route must also assemble compact V3.
                let mut v3_route = route.clone();
                let hops = if buy { &mut v3_route.buy_hops } else { &mut v3_route.sell_hops };
                let index = if buy { 1 } else { 0 };
                let PumpFunQuoteVenue::PumpFun {
                    mint,
                    curve,
                    quote_token_program,
                    base_transfer_fee,
                    quote_transfer_fee,
                } = hops[index].venue.clone()
                else {
                    unreachable!()
                };
                hops[index].venue = PumpFunQuoteVenue::PumpFunV3 {
                    mint,
                    params: crate::PumpFunV3Params {
                        curve,
                        quote_token_program,
                        base_transfer_fee,
                        quote_transfer_fee,
                        minimum_output_amount: 1,
                    },
                };
                let mut v3_params = params.clone();
                v3_params.protocol_params = DexParamEnum::PumpFunQuoteRoute(v3_route);
                let compact = if buy {
                    PumpFunInstructionBuilder.build_buy_instructions(&v3_params).await
                } else {
                    PumpFunInstructionBuilder.build_sell_instructions(&v3_params).await
                }
                .unwrap();
                let v3_ix = compact.iter().find(|ix| ix.program_id == accounts::PUMPFUN).unwrap();
                assert_eq!(v3_ix.accounts.len(), 17);
                assert_eq!(
                    &v3_ix.data[..8],
                    if buy {
                        &[225, 247, 80, 30, 213, 179, 132, 136]
                    } else {
                        &[28, 146, 222, 119, 38, 196, 105, 213]
                    }
                );
                assert_eq!(&v3_ix.data[8..24], &ix.data[8..24]); // Hop minimum, not params' placeholder.
                if buy {
                    assert_eq!(&v3_ix.data[24..], &[0]);
                } else {
                    assert_eq!(v3_ix.data.len(), 24);
                }
                let mapping = if buy {
                    [0, 1, 2, 3, 4, 10, 11, 12, 13, 14, 15, 20, 22, 9, 24, 25, 26]
                } else {
                    [0, 1, 2, 3, 4, 10, 11, 12, 13, 14, 15, 19, 21, 9, 23, 24, 25]
                };
                for (i, old) in mapping.into_iter().enumerate() {
                    // Buyback recipient is independently selected for each instruction.
                    if i != 13 {
                        assert_eq!(v3_ix.accounts[i], ix.accounts[old]);
                    }
                }
                assert!(!compact
                    .iter()
                    .any(|ix| ix.program_id == TOKEN_PROGRAM_2022 && ix.data == [9]));
                // Reject unsafe snapshots and unprotected/oversized route legs.
                for invalid in 0..7 {
                    let mut bad = route.clone();
                    let hops = if buy { &mut bad.buy_hops } else { &mut bad.sell_hops };
                    let curve_hop = if buy { 1 } else { 0 };
                    if invalid == 5 {
                        hops[curve_hop].minimum_amount_out = 0;
                    } else if invalid == 6 {
                        hops[1].input_amount = Some(901);
                    } else {
                        let PumpFunQuoteVenue::PumpFun {
                            curve,
                            quote_token_program,
                            quote_transfer_fee,
                            ..
                        } = &mut hops[curve_hop].venue
                        else {
                            unreachable!()
                        };
                        match invalid {
                            0 => Arc::make_mut(&mut curve.bonding_curve).complete = true,
                            1 => {
                                Arc::make_mut(&mut curve.bonding_curve).account =
                                    Pubkey::new_unique()
                            }
                            2 => curve.quote_mint = Pubkey::new_unique(),
                            3 => *quote_token_program = Pubkey::new_unique(),
                            4 => quote_transfer_fee.basis_points = 1,
                            _ => unreachable!(),
                        }
                    }
                    assert!(bad
                        .preview(params.input_mint, params.output_mint, 1000, 100, buy)
                        .is_err());
                }
                params.fixed_output_amount = Some(800);
                let result = if buy {
                    PumpFunInstructionBuilder.build_buy_instructions(&params).await
                } else {
                    PumpFunInstructionBuilder.build_sell_instructions(&params).await
                };
                assert!(result.is_err());
            }
        }
    }

    #[tokio::test]
    async fn v3_direct_limits_and_account_validation() {
        use crate::trading::core::params::TokenTransferFee;
        for native in [false, true] {
            let mint = Pubkey::new_unique();
            let mut params = swap_params_for_buy(mint, TOKEN_PROGRAM_2022);
            let DexParamEnum::PumpFun(mut curve) = params.protocol_params.clone() else {
                unreachable!()
            };
            let quote =
                if native { crate::constants::WSOL_TOKEN_ACCOUNT } else { Pubkey::new_unique() };
            curve.quote_mint = quote;
            Arc::make_mut(&mut curve.bonding_curve).quote_mint = quote;
            let v3 = crate::PumpFunV3Params {
                curve,
                quote_token_program: if native { TOKEN_PROGRAM } else { TOKEN_PROGRAM_2022 },
                minimum_output_amount: 80,
                base_transfer_fee: TokenTransferFee::default(),
                quote_transfer_fee: TokenTransferFee::default(),
            };
            params.protocol_params = DexParamEnum::PumpFunV3(v3.clone());
            params.input_amount = Some(100);
            params.input_mint = if native { crate::constants::SOL_TOKEN_ACCOUNT } else { quote };
            params.open_seed_optimize = true; // V3 uses canonical ATAs even with this preference.
            let assert_compact_parity = |ix: &Instruction, name: &str, amount, limit, fill| {
                use super::super::pump_compact_accounts::{
                    derive_pump_v3_accounts, PumpCompactAccountParams,
                };
                use super::super::pump_upgrade::build_pump_upgrade_instruction;
                let expected = global_constants::BUYBACK_FEE_RECIPIENTS
                    .iter()
                    .find_map(|recipient| {
                        let keys = derive_pump_v3_accounts(PumpCompactAccountParams {
                            user: params.payer.pubkey(),
                            base_mint: mint,
                            quote_mint: quote,
                            base_token_program: TOKEN_PROGRAM_2022,
                            quote_token_program: v3.quote_token_program,
                            buyback_recipient: *recipient,
                            cashback: false,
                            complete: false,
                        })
                        .unwrap();
                        (keys["buyback_fee_recipient"] == ix.accounts[13].pubkey).then_some(keys)
                    })
                    .expect("buyback recipient belongs to the configured set");
                assert_eq!(
                    *ix,
                    build_pump_upgrade_instruction(name, &expected, &[amount, limit], fill, &[])
                        .unwrap()
                );
            };
            for exact_out in [false, true] {
                params.fixed_output_amount = exact_out.then_some(70);
                let ixs = PumpFunInstructionBuilder.build_buy_instructions(&params).await.unwrap();
                let ix = ixs.iter().find(|ix| ix.program_id == accounts::PUMPFUN).unwrap();
                assert_eq!(ix.accounts.len(), 17);
                assert_eq!(
                    &ix.data[..8],
                    if exact_out {
                        &[7, 5, 29, 196, 245, 23, 101, 80]
                    } else {
                        &[225, 247, 80, 30, 213, 179, 132, 136]
                    }
                );
                assert_eq!(
                    u64::from_le_bytes(ix.data[8..16].try_into().unwrap()),
                    if exact_out { 70 } else { 100 }
                );
                assert_eq!(
                    u64::from_le_bytes(ix.data[16..24].try_into().unwrap()),
                    if exact_out { 100 } else { 80 }
                );
                assert_eq!(ix.data[24], 0);
                assert_compact_parity(
                    ix,
                    if exact_out { "pump_buy_v3" } else { "pump_buy_exact_quote_in_v3" },
                    if exact_out { 70 } else { 100 },
                    if exact_out { 100 } else { 80 },
                    Some(false),
                );
                assert_eq!(ix.accounts[9].pubkey, crate::common::spl_associated_token_account::get_associated_token_address_with_program_id(&params.payer.pubkey(), &mint, &TOKEN_PROGRAM_2022));
                if native {
                    assert!(!ixs.iter().any(|i| i.program_id == TOKEN_PROGRAM));
                }
            }
            params.fixed_output_amount = None;
            std::mem::swap(&mut params.input_mint, &mut params.output_mint);
            let ixs = PumpFunInstructionBuilder.build_sell_instructions(&params).await.unwrap();
            let ix = ixs.iter().find(|ix| ix.program_id == accounts::PUMPFUN).unwrap();
            assert_eq!(ix.data.len(), 24);
            assert_eq!(u64::from_le_bytes(ix.data[16..24].try_into().unwrap()), 80);
            assert_compact_parity(ix, "pump_sell_v3", 100, 80, None);
            for invalid in 0..5 {
                let mut bad = v3.clone();
                match invalid {
                    0 => Arc::make_mut(&mut bad.curve.bonding_curve).is_cashback_coin = true,
                    1 => Arc::make_mut(&mut bad.curve.bonding_curve).complete = true,
                    2 => bad.base_transfer_fee.basis_points = 1,
                    3 => bad.minimum_output_amount = 0,
                    4 => bad.quote_token_program = Pubkey::new_unique(),
                    _ => unreachable!(),
                }
                params.protocol_params = DexParamEnum::PumpFunV3(bad);
                assert!(PumpFunInstructionBuilder.build_sell_instructions(&params).await.is_err());
            }
        }
    }

    #[test]
    fn pump_suffix_buy_forces_token_2022_even_with_explicit_legacy_token_program() {
        crate::common::seed::set_default_rents();
        let params = swap_params_for_buy(pump_mint(), TOKEN_PROGRAM);
        let instructions = build_buy(&params).unwrap();

        assert_eq!(instructions.len(), 2);
        assert_eq!(instructions[1].accounts[8].pubkey, TOKEN_PROGRAM_2022);
        assert_eq!(instructions[0].program_id, crate::constants::ASSOCIATED_TOKEN_PROGRAM_ID);
        assert_eq!(instructions[0].accounts[5].pubkey, TOKEN_PROGRAM_2022);
        assert_eq!(instructions[1].accounts.len(), 18);
        assert_eq!(instructions[1].data.len(), 25);
    }

    #[test]
    fn pump_suffix_sell_forces_token_2022_even_with_explicit_legacy_token_program() {
        crate::common::seed::set_default_rents();
        let mut params = swap_params_for_buy(pump_mint(), TOKEN_PROGRAM);
        params.trade_type = crate::swqos::TradeType::Sell;
        params.input_mint = pump_mint();
        params.output_mint = crate::constants::SOL_TOKEN_ACCOUNT;
        params.create_output_mint_ata = false;

        let instructions = build_sell(&params).unwrap();
        let ix = instructions.last().unwrap();

        assert_eq!(ix.accounts[9].pubkey, TOKEN_PROGRAM_2022);
    }

    #[test]
    fn non_pump_buy_respects_explicit_legacy_token_program() {
        crate::common::seed::set_default_rents();
        let params = swap_params_for_buy(Pubkey::new_unique(), TOKEN_PROGRAM);
        let instructions = build_buy(&params).unwrap();

        assert_eq!(instructions[2].accounts[8].pubkey, TOKEN_PROGRAM);
    }

    #[test]
    fn pumpfun_v1_fixed_output_uses_buy_with_max_input_budget() {
        let mut params = swap_params_for_buy(pump_mint(), TOKEN_PROGRAM);
        params.create_output_mint_ata = false;
        params.fixed_output_amount = Some(42);
        params.use_exact_sol_amount = Some(true);

        let instructions = build_buy(&params).unwrap();
        let ix = instructions.last().unwrap();

        assert_eq!(&ix.data[..8], crate::instruction::utils::pumpfun::BUY_DISCRIMINATOR);
        assert_eq!(ix.data.len(), 25);
        assert_eq!(u64::from_le_bytes(ix.data[8..16].try_into().unwrap()), 42);
        assert_eq!(
            u64::from_le_bytes(ix.data[16..24].try_into().unwrap()),
            params.input_amount.unwrap()
        );
        assert_eq!(ix.data[24], 0);
    }

    #[test]
    fn pumpfun_v2_usdc_fixed_output_uses_buy_with_max_input_budget() {
        let mut params = swap_params_for_buy(pump_mint(), TOKEN_PROGRAM);
        params.create_output_mint_ata = false;
        params.input_mint = crate::constants::USDC_TOKEN_ACCOUNT;
        params.fixed_output_amount = Some(42);
        params.use_exact_sol_amount = Some(true);
        if let DexParamEnum::PumpFun(protocol_params) = &mut params.protocol_params {
            *protocol_params =
                protocol_params.clone().with_quote_mint(crate::constants::USDC_TOKEN_ACCOUNT);
        }

        let instructions = build_buy(&params).unwrap();
        let ix = instructions.last().unwrap();

        assert_eq!(&ix.data[..8], crate::instruction::utils::pumpfun::BUY_V2_DISCRIMINATOR);
        assert_eq!(u64::from_le_bytes(ix.data[8..16].try_into().unwrap()), 42);
        assert_eq!(
            u64::from_le_bytes(ix.data[16..24].try_into().unwrap()),
            params.input_amount.unwrap()
        );
    }

    #[test]
    fn pumpfun_wsol_quote_regular_buy_uses_v1_native_sol() {
        let mut params = swap_params_for_buy(pump_mint(), TOKEN_PROGRAM);
        params.create_output_mint_ata = false;
        params.create_input_mint_ata = true;
        params.close_input_mint_ata = true;
        params.use_exact_sol_amount = Some(false);
        if let DexParamEnum::PumpFun(protocol_params) = &mut params.protocol_params {
            *protocol_params =
                protocol_params.clone().with_quote_mint(crate::constants::WSOL_TOKEN_ACCOUNT);
        }

        let instructions = build_buy(&params).unwrap();
        assert_eq!(instructions.len(), 1);
        let buy_ix = instructions.last().unwrap();
        let expected = crate::utils::calc::common::calculate_with_slippage_buy(
            params.input_amount.unwrap(),
            params.slippage_basis_points.unwrap(),
        );

        assert_eq!(&buy_ix.data[..8], crate::instruction::utils::pumpfun::BUY_DISCRIMINATOR);
        assert_eq!(u64::from_le_bytes(buy_ix.data[16..24].try_into().unwrap()), expected);
        assert_eq!(buy_ix.accounts.len(), 18);
    }

    #[test]
    fn pumpfun_wsol_quote_regular_buy_skips_wsol_ata_create_on_hot_path() {
        let mut params = swap_params_for_buy(pump_mint(), TOKEN_PROGRAM);
        params.create_output_mint_ata = false;
        params.create_input_mint_ata = true;
        params.close_input_mint_ata = true;
        params.use_exact_sol_amount = Some(false);
        if let DexParamEnum::PumpFun(protocol_params) = &mut params.protocol_params {
            *protocol_params =
                protocol_params.clone().with_quote_mint(crate::constants::WSOL_TOKEN_ACCOUNT);
        }

        let instructions = build_buy(&params).unwrap();

        assert_eq!(instructions.len(), 1);
        assert_eq!(
            &instructions.last().unwrap().data[..8],
            crate::instruction::utils::pumpfun::BUY_DISCRIMINATOR
        );
    }

    #[test]
    fn pumpfun_v2_explicit_wsol_input_with_output_ata_create_fits_without_dropping_priority() {
        let mut params = swap_params_for_buy(pump_mint(), TOKEN_PROGRAM);
        params.input_mint = crate::constants::WSOL_TOKEN_ACCOUNT;
        params.create_input_mint_ata = true;
        params.create_output_mint_ata = true;
        params.use_exact_sol_amount = Some(false);
        if let DexParamEnum::PumpFun(protocol_params) = &mut params.protocol_params {
            *protocol_params =
                protocol_params.clone().with_quote_mint(crate::constants::WSOL_TOKEN_ACCOUNT);
        }

        let business_instructions = build_buy(&params).unwrap();
        let transaction = crate::trading::common::transaction_builder::build_transaction(
            &params.payer,
            150_000,
            500_000,
            &business_instructions,
            &[],
            Some(solana_hash::Hash::new_unique()),
            None,
            "PumpFun",
            true,
            true,
            &Pubkey::new_unique(),
            0.001,
            None,
        )
        .unwrap();
        assert!(wincode::serialize(&transaction).unwrap().len() <= 1232);
        assert_eq!(transaction.message.instructions().len(), business_instructions.len() + 3);
    }

    #[test]
    fn pumpfun_v2_explicit_wsol_input_hot_path_transaction_fits_when_output_ata_prepared() {
        let mut params = swap_params_for_buy(pump_mint(), TOKEN_PROGRAM);
        params.input_mint = crate::constants::WSOL_TOKEN_ACCOUNT;
        params.create_input_mint_ata = true;
        params.create_output_mint_ata = false;
        params.use_exact_sol_amount = Some(false);
        if let DexParamEnum::PumpFun(protocol_params) = &mut params.protocol_params {
            *protocol_params =
                protocol_params.clone().with_quote_mint(crate::constants::WSOL_TOKEN_ACCOUNT);
        }

        let business_instructions = build_buy(&params).unwrap();
        let transaction = crate::trading::common::transaction_builder::build_transaction(
            &params.payer,
            150_000,
            500_000,
            &business_instructions,
            &[],
            Some(solana_hash::Hash::new_unique()),
            None,
            "PumpFun",
            true,
            true,
            &Pubkey::new_unique(),
            0.001,
            None,
        )
        .unwrap();
        let serialized = wincode::serialize(&transaction).unwrap();

        assert!(
            serialized.len() <= 1232,
            "serialized transaction too large: {} > 1232",
            serialized.len()
        );
    }

    #[test]
    fn pumpfun_from_trade_wsol_quote_regular_buy_selects_v1() {
        let mint = pump_mint();
        let mut params = swap_params_for_buy(mint, TOKEN_PROGRAM);
        params.create_output_mint_ata = false;
        params.use_exact_sol_amount = Some(false);
        if let DexParamEnum::PumpFun(protocol_params) = &mut params.protocol_params {
            *protocol_params = PumpFunParams::from_trade(
                Pubkey::default(),
                Pubkey::default(),
                mint,
                crate::constants::WSOL_TOKEN_ACCOUNT,
                protocol_params.effective_creator_for_trade(),
                protocol_params.creator_vault,
                protocol_params.bonding_curve.virtual_token_reserves,
                protocol_params.bonding_curve.virtual_sol_reserves,
                protocol_params.bonding_curve.real_token_reserves,
                protocol_params.bonding_curve.real_sol_reserves,
                None,
                protocol_params.fee_recipient,
                TOKEN_PROGRAM,
                false,
                Some(false),
            );
        }

        let instructions = build_buy(&params).unwrap();
        let buy_ix = instructions.last().unwrap();

        assert_eq!(&buy_ix.data[..8], crate::instruction::utils::pumpfun::BUY_DISCRIMINATOR);
        assert_eq!(buy_ix.accounts.len(), 18);
    }

    #[test]
    fn pumpfun_v2_wsol_input_does_not_auto_wrap_when_ata_create_disabled() {
        let mut params = swap_params_for_buy(pump_mint(), TOKEN_PROGRAM);
        params.input_mint = crate::constants::WSOL_TOKEN_ACCOUNT;
        params.create_output_mint_ata = false;
        params.create_input_mint_ata = false;
        params.use_exact_sol_amount = Some(false);
        if let DexParamEnum::PumpFun(protocol_params) = &mut params.protocol_params {
            *protocol_params =
                protocol_params.clone().with_quote_mint(crate::constants::WSOL_TOKEN_ACCOUNT);
        }

        let instructions = build_buy(&params).unwrap();

        assert_eq!(instructions.len(), 1);
        assert_eq!(
            &instructions[0].data[..8],
            crate::instruction::utils::pumpfun::BUY_V2_DISCRIMINATOR
        );
        assert_eq!(instructions[0].accounts.len(), 27);
    }

    #[test]
    fn pumpfun_sell_does_not_select_v2_from_base_mint_wsol() {
        let mut params = swap_params_for_buy(crate::constants::WSOL_TOKEN_ACCOUNT, TOKEN_PROGRAM);
        params.trade_type = crate::swqos::TradeType::Sell;
        params.input_mint = crate::constants::WSOL_TOKEN_ACCOUNT;
        params.output_mint = crate::constants::SOL_TOKEN_ACCOUNT;
        params.create_output_mint_ata = false;
        if let DexParamEnum::PumpFun(protocol_params) = &mut params.protocol_params {
            *protocol_params =
                protocol_params.clone().with_quote_mint(crate::constants::WSOL_TOKEN_ACCOUNT);
        }

        let instructions = build_sell(&params).unwrap();
        let ix = instructions.last().unwrap();

        assert_eq!(&ix.data[..8], crate::instruction::utils::pumpfun::SELL_DISCRIMINATOR);
    }

    #[test]
    fn pumpfun_sell_selects_v2_for_explicit_wsol_output_settlement() {
        let mint = pump_mint();
        let mut params = swap_params_for_buy(mint, TOKEN_PROGRAM);
        params.trade_type = crate::swqos::TradeType::Sell;
        params.input_mint = mint;
        params.output_mint = crate::constants::WSOL_TOKEN_ACCOUNT;
        params.create_output_mint_ata = false;
        if let DexParamEnum::PumpFun(protocol_params) = &mut params.protocol_params {
            *protocol_params =
                protocol_params.clone().with_quote_mint(crate::constants::WSOL_TOKEN_ACCOUNT);
        }

        let instructions = build_sell(&params).unwrap();
        let ix = instructions.last().unwrap();

        assert_eq!(&ix.data[..8], crate::instruction::utils::pumpfun::SELL_V2_DISCRIMINATOR);
        assert_eq!(ix.accounts.len(), 26);
    }

    #[test]
    fn pumpfun_usdc_dev_trade_uses_usdc_initial_quote_reserves() {
        let mint = pump_mint();
        let dev_quote_amount = 707_080;
        let params = PumpFunParams::from_dev_trade_with_quote_mint(
            mint,
            55_855_975_892_641,
            dev_quote_amount,
            Pubkey::new_unique(),
            Pubkey::default(),
            Pubkey::default(),
            Pubkey::default(),
            None,
            global_constants::FEE_RECIPIENT,
            TOKEN_PROGRAM,
            false,
            Some(false),
            crate::constants::USDC_TOKEN_ACCOUNT,
        );

        assert_eq!(params.quote_mint, crate::constants::USDC_TOKEN_ACCOUNT);
        assert_eq!(params.bonding_curve.quote_mint, crate::constants::USDC_TOKEN_ACCOUNT);
        assert_eq!(
            params.bonding_curve.virtual_sol_reserves,
            global_constants::INITIAL_VIRTUAL_USDC_RESERVES + dev_quote_amount
        );
    }

    #[test]
    fn pumpfun_with_quote_mint_adjusts_dev_trade_quote_reserves() {
        let mint = pump_mint();
        let dev_quote_amount = 707_080;
        let params = PumpFunParams::from_dev_trade(
            mint,
            55_855_975_892_641,
            dev_quote_amount,
            Pubkey::new_unique(),
            Pubkey::default(),
            Pubkey::default(),
            Pubkey::default(),
            None,
            global_constants::FEE_RECIPIENT,
            TOKEN_PROGRAM,
            false,
            Some(false),
        )
        .with_quote_mint(crate::constants::USDC_TOKEN_ACCOUNT);

        assert_eq!(
            params.bonding_curve.virtual_sol_reserves,
            global_constants::INITIAL_VIRTUAL_USDC_RESERVES + dev_quote_amount
        );
    }

    #[test]
    fn pumpfun_usdc_trade_event_preserves_virtual_quote_reserves() {
        let mint = pump_mint();
        let virtual_quote_reserves = 4_527_693_121;
        let real_quote_reserves = 235_693_121;
        let params = PumpFunParams::from_trade(
            Pubkey::default(),
            Pubkey::default(),
            mint,
            crate::constants::USDC_TOKEN_ACCOUNT,
            Pubkey::new_unique(),
            Pubkey::default(),
            944_144_024_107_359,
            virtual_quote_reserves,
            737_244_024_107_359,
            real_quote_reserves,
            None,
            global_constants::FEE_RECIPIENT,
            TOKEN_PROGRAM,
            false,
            Some(false),
        );

        assert_eq!(params.quote_mint, crate::constants::USDC_TOKEN_ACCOUNT);
        assert_eq!(params.bonding_curve.virtual_quote_reserves(), virtual_quote_reserves);
        assert_eq!(params.bonding_curve.real_quote_reserves(), real_quote_reserves);
    }

    #[test]
    fn pumpfun_solscan_sol_quote_mint_keeps_legacy_layout() {
        let mint = pump_mint();
        let params = PumpFunParams::from_trade(
            Pubkey::default(),
            Pubkey::default(),
            mint,
            crate::constants::SOL_TOKEN_ACCOUNT,
            Pubkey::new_unique(),
            Pubkey::default(),
            944_144_024_107_359,
            30_235_693_121,
            737_244_024_107_359,
            235_693_121,
            None,
            global_constants::FEE_RECIPIENT,
            TOKEN_PROGRAM,
            false,
            Some(false),
        );

        assert_eq!(params.quote_mint, Pubkey::default());
        assert_eq!(params.bonding_curve.quote_mint, crate::constants::WSOL_TOKEN_ACCOUNT);
    }

    #[test]
    fn pumpfun_with_solscan_sol_quote_mint_selects_v1() {
        let mut params = swap_params_for_buy(pump_mint(), TOKEN_PROGRAM);
        params.create_output_mint_ata = false;
        if let DexParamEnum::PumpFun(protocol_params) = &mut params.protocol_params {
            *protocol_params =
                protocol_params.clone().with_quote_mint(crate::constants::SOL_TOKEN_ACCOUNT);
            assert_eq!(protocol_params.quote_mint, Pubkey::default());
            assert_eq!(
                protocol_params.bonding_curve.quote_mint,
                crate::constants::WSOL_TOKEN_ACCOUNT
            );
        }

        let ix = build_buy(&params).unwrap().pop().unwrap();
        assert_eq!(
            &ix.data[..8],
            crate::instruction::utils::pumpfun::BUY_EXACT_SOL_IN_DISCRIMINATOR
        );
        assert_eq!(ix.accounts.len(), 18);
    }

    #[test]
    fn pumpfun_rpc_sol_sentinel_quote_mint_selects_v1() {
        let mut params = swap_params_for_buy(pump_mint(), TOKEN_PROGRAM);
        params.create_output_mint_ata = false;
        if let DexParamEnum::PumpFun(protocol_params) = &mut params.protocol_params {
            protocol_params.quote_mint = crate::constants::SOL_TOKEN_ACCOUNT;
            assert_eq!(protocol_params.quote_mint, crate::constants::SOL_TOKEN_ACCOUNT);
        }

        let ix = build_buy(&params).unwrap().pop().unwrap();
        assert_eq!(
            &ix.data[..8],
            crate::instruction::utils::pumpfun::BUY_EXACT_SOL_IN_DISCRIMINATOR
        );
        assert_eq!(ix.accounts.len(), 18);
    }

    #[test]
    fn pumpfun_v2_usdc_buy_rejects_sol_input() {
        let mut params = swap_params_for_buy(pump_mint(), TOKEN_PROGRAM);
        params.create_output_mint_ata = false;
        params.input_mint = crate::constants::SOL_TOKEN_ACCOUNT;
        if let DexParamEnum::PumpFun(protocol_params) = &mut params.protocol_params {
            *protocol_params =
                protocol_params.clone().with_quote_mint(crate::constants::USDC_TOKEN_ACCOUNT);
        }

        let err = build_buy(&params).unwrap_err().to_string();
        assert!(err.contains("USDC quote pools must be bought with USDC"));
    }

    #[test]
    fn pumpfun_usdc_quote_mint_selects_v2_without_global_flag() {
        let mut params = swap_params_for_buy(pump_mint(), TOKEN_PROGRAM);
        params.create_output_mint_ata = false;
        params.input_mint = crate::constants::USDC_TOKEN_ACCOUNT;
        if let DexParamEnum::PumpFun(protocol_params) = &mut params.protocol_params {
            *protocol_params =
                protocol_params.clone().with_quote_mint(crate::constants::USDC_TOKEN_ACCOUNT);
        }

        let ix = build_buy(&params).unwrap().pop().unwrap();
        assert_eq!(
            &ix.data[..8],
            crate::instruction::utils::pumpfun::BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR
        );
        assert_eq!(ix.accounts.len(), 27);
    }

    #[test]
    fn pumpfun_v2_usdc_buy_uses_idl_accounts_17_18_19() {
        let mint = pump_mint();
        let mut params = swap_params_for_buy(mint, TOKEN_PROGRAM);
        params.create_output_mint_ata = false;
        params.input_mint = crate::constants::USDC_TOKEN_ACCOUNT;
        if let DexParamEnum::PumpFun(protocol_params) = &mut params.protocol_params {
            *protocol_params =
                protocol_params.clone().with_quote_mint(crate::constants::USDC_TOKEN_ACCOUNT);
        }

        let associated_creator_vault =
            if let DexParamEnum::PumpFun(protocol_params) = &params.protocol_params {
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &protocol_params.creator_vault,
                    &crate::constants::USDC_TOKEN_ACCOUNT,
                    &crate::constants::TOKEN_PROGRAM,
                )
            } else {
                Pubkey::default()
            };
        let ix = build_buy(&params).unwrap().pop().unwrap();
        assert_eq!(ix.accounts.len(), 27);
        assert_eq!(ix.accounts[17].pubkey, associated_creator_vault);
        assert_eq!(
            ix.accounts[18].pubkey,
            crate::instruction::utils::pumpfun::get_fee_sharing_config_pda(&mint).unwrap()
        );
        assert_eq!(ix.accounts[19].pubkey, accounts::GLOBAL_VOLUME_ACCUMULATOR);
        assert_eq!(ix.accounts[22].pubkey, accounts::FEE_CONFIG);
    }

    #[test]
    fn pumpfun_v2_usdc_buyback_fee_recipient_is_writable() {
        let mint = pump_mint();
        let mut params = swap_params_for_buy(mint, TOKEN_PROGRAM);
        params.create_output_mint_ata = false;
        params.input_mint = crate::constants::USDC_TOKEN_ACCOUNT;
        if let DexParamEnum::PumpFun(protocol_params) = &mut params.protocol_params {
            *protocol_params =
                protocol_params.clone().with_quote_mint(crate::constants::USDC_TOKEN_ACCOUNT);
        }

        let buy_ix = build_buy(&params).unwrap().pop().unwrap();
        assert!(global_constants::BUYBACK_FEE_RECIPIENTS.contains(&buy_ix.accounts[8].pubkey));
        assert!(buy_ix.accounts[8].is_writable);

        params.trade_type = crate::swqos::TradeType::Sell;
        params.input_mint = mint;
        params.output_mint = crate::constants::USDC_TOKEN_ACCOUNT;
        let sell_ix = build_sell(&params).unwrap().pop().unwrap();
        assert!(global_constants::BUYBACK_FEE_RECIPIENTS.contains(&sell_ix.accounts[8].pubkey));
        assert!(sell_ix.accounts[8].is_writable);
    }

    #[test]
    fn pumpfun_v1_sell_fixed_output_uses_min_sol_directly() {
        let mint = pump_mint();
        let mut params = swap_params_for_buy(mint, TOKEN_PROGRAM);
        params.trade_type = crate::swqos::TradeType::Sell;
        params.input_mint = mint;
        params.output_mint = crate::constants::SOL_TOKEN_ACCOUNT;
        params.create_output_mint_ata = false;
        params.fixed_output_amount = Some(42);

        let instructions = build_sell(&params).unwrap();
        let ix = instructions.last().unwrap();

        assert_eq!(&ix.data[..8], crate::instruction::utils::pumpfun::SELL_DISCRIMINATOR);
        assert_eq!(ix.data.len(), 24);
        assert_eq!(
            u64::from_le_bytes(ix.data[8..16].try_into().unwrap()),
            params.input_amount.unwrap()
        );
        assert_eq!(u64::from_le_bytes(ix.data[16..24].try_into().unwrap()), 42);
    }

    #[test]
    fn pumpfun_v2_usdc_sell_fixed_output_uses_min_quote_directly() {
        let mint = pump_mint();
        let mut params = swap_params_for_buy(mint, TOKEN_PROGRAM);
        params.trade_type = crate::swqos::TradeType::Sell;
        params.input_mint = mint;
        params.output_mint = crate::constants::USDC_TOKEN_ACCOUNT;
        params.create_output_mint_ata = false;
        params.fixed_output_amount = Some(42);
        if let DexParamEnum::PumpFun(protocol_params) = &mut params.protocol_params {
            *protocol_params =
                protocol_params.clone().with_quote_mint(crate::constants::USDC_TOKEN_ACCOUNT);
        }

        let instructions = build_sell(&params).unwrap();
        let ix = instructions.last().unwrap();

        assert_eq!(&ix.data[..8], crate::instruction::utils::pumpfun::SELL_V2_DISCRIMINATOR);
        assert_eq!(ix.data.len(), 24);
        assert_eq!(
            u64::from_le_bytes(ix.data[8..16].try_into().unwrap()),
            params.input_amount.unwrap()
        );
        assert_eq!(u64::from_le_bytes(ix.data[16..24].try_into().unwrap()), 42);
    }
}
