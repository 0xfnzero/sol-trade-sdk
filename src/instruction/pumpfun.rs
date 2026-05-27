//! Pump.fun bonding-curve swap ix assembly ([`SwapParams`](crate::trading::core::params::SwapParams)).
//!
//! The SDK selects the legacy or V2 on-chain layout from `PumpFunParams.quote_mint`.
//! `Pubkey::default()` and the Solscan SOL sentinel keep the smaller legacy SOL layout;
//! explicit WSOL or USDC uses the V2 27/26-account unified metas.
//! Default (`false`) keeps the smaller legacy SOL-paired instruction layout for latency.

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
fn effective_pump_mint_token_program(protocol_params: &PumpFunParams) -> Pubkey {
    let tp = protocol_params.token_program;
    if tp == Pubkey::default() {
        TOKEN_PROGRAM_2022
    } else {
        tp
    }
}

/// Resolve quote mint and its token program from PumpFunParams.
/// `Pubkey::default()` / `SOL_TOKEN_ACCOUNT` means legacy SOL-paired; use WSOL mint when a
/// downstream V2 helper needs a concrete SPL quote mint.
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
        build_buy(params)
    }

    async fn build_sell_instructions(&self, params: &SwapParams) -> Result<Vec<Instruction>> {
        build_sell(params)
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
    let explicit_v2_quote = protocol_params.quote_mint != Pubkey::default()
        && protocol_params.quote_mint != crate::constants::SOL_TOKEN_ACCOUNT;
    Ok(explicit_v2_quote || !is_sol_quote_mint(&quote_mint))
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
    let token_program = effective_pump_mint_token_program(protocol_params);
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
    let token_program = effective_pump_mint_token_program(protocol_params);
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
    let base_token_program = effective_pump_mint_token_program(protocol_params);
    let base_token_program_meta = if base_token_program == TOKEN_PROGRAM_2022 {
        crate::constants::TOKEN_PROGRAM_2022_META
    } else {
        crate::constants::TOKEN_PROGRAM_META
    };

    let (quote_mint, quote_token_program) = effective_quote_mint_and_token_program(protocol_params);
    validate_v2_buy_quote_mint(&params.input_mint, &quote_mint)?;
    let quote_token_program_meta = crate::constants::TOKEN_PROGRAM_META;

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

    let (buy_data, quote_amount_to_fund) = if let Some(token_amount) = params.fixed_output_amount {
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

    if params.create_input_mint_ata {
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

    if params.close_input_mint_ata {
        push_close_wsol_if_needed(&mut instructions, &params.payer.pubkey(), &quote_mint);
    }

    Ok(instructions)
}

fn build_sell_unified(params: &SwapParams) -> Result<Vec<Instruction>> {
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
    let base_token_program = effective_pump_mint_token_program(protocol_params);
    let base_token_program_meta = if base_token_program == TOKEN_PROGRAM_2022 {
        crate::constants::TOKEN_PROGRAM_2022_META
    } else {
        crate::constants::TOKEN_PROGRAM_META
    };

    let (quote_mint, quote_token_program) = effective_quote_mint_and_token_program(protocol_params);
    validate_v2_sell_quote_mint(&params.output_mint, &quote_mint)?;
    let quote_token_program_meta = crate::constants::TOKEN_PROGRAM_META;

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
            address_lookup_table_account: None,
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
            grpc_recv_us: None,
            use_exact_sol_amount: Some(true),
        }
    }

    #[test]
    fn test_claim_cashback_instruction() {
        let payer = Pubkey::new_unique();
        let ix = claim_cashback_pumpfun_instruction(&payer).unwrap();
        assert_eq!(ix.accounts.len(), 3);
        assert_eq!(ix.accounts[0].pubkey, payer);
    }

    #[test]
    fn pump_suffix_buy_respects_explicit_legacy_token_program() {
        crate::common::seed::set_default_rents();
        let params = swap_params_for_buy(pump_mint(), TOKEN_PROGRAM);
        let instructions = build_buy(&params).unwrap();

        assert_eq!(instructions.len(), 3);
        assert_eq!(instructions[2].accounts[8].pubkey, TOKEN_PROGRAM);
        assert_eq!(instructions[1].program_id, TOKEN_PROGRAM);
        assert_eq!(instructions[2].accounts.len(), 18);
        assert_eq!(instructions[2].data.len(), 25);
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
    fn pumpfun_v2_fixed_output_uses_buy_with_max_input_budget() {
        let mut params = swap_params_for_buy(pump_mint(), TOKEN_PROGRAM);
        params.create_output_mint_ata = false;
        params.fixed_output_amount = Some(42);
        params.use_exact_sol_amount = Some(true);
        if let DexParamEnum::PumpFun(protocol_params) = &mut params.protocol_params {
            *protocol_params =
                protocol_params.clone().with_quote_mint(crate::constants::WSOL_TOKEN_ACCOUNT);
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
    fn pumpfun_v2_regular_buy_wraps_max_quote_budget() {
        let mut params = swap_params_for_buy(pump_mint(), TOKEN_PROGRAM);
        params.create_output_mint_ata = false;
        params.create_input_mint_ata = true;
        params.use_exact_sol_amount = Some(false);
        if let DexParamEnum::PumpFun(protocol_params) = &mut params.protocol_params {
            *protocol_params =
                protocol_params.clone().with_quote_mint(crate::constants::WSOL_TOKEN_ACCOUNT);
        }

        let instructions = build_buy(&params).unwrap();
        let transfer_ix = &instructions[1];
        let system_ix = bincode::deserialize::<
            solana_system_interface::instruction::SystemInstruction,
        >(&transfer_ix.data)
        .unwrap();
        let expected = crate::utils::calc::common::calculate_with_slippage_buy(
            params.input_amount.unwrap(),
            params.slippage_basis_points.unwrap(),
        );

        match system_ix {
            solana_system_interface::instruction::SystemInstruction::Transfer { lamports } => {
                assert_eq!(lamports, expected);
            }
            other => panic!("unexpected system instruction: {:?}", other),
        }
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
    fn pumpfun_v2_buyback_fee_recipient_is_writable() {
        let mint = pump_mint();
        let mut params = swap_params_for_buy(mint, TOKEN_PROGRAM);
        params.create_output_mint_ata = false;
        if let DexParamEnum::PumpFun(protocol_params) = &mut params.protocol_params {
            *protocol_params =
                protocol_params.clone().with_quote_mint(crate::constants::WSOL_TOKEN_ACCOUNT);
        }

        let buy_ix = build_buy(&params).unwrap().pop().unwrap();
        assert!(global_constants::BUYBACK_FEE_RECIPIENTS.contains(&buy_ix.accounts[8].pubkey));
        assert!(buy_ix.accounts[8].is_writable);

        params.trade_type = crate::swqos::TradeType::Sell;
        params.input_mint = mint;
        params.output_mint = crate::constants::SOL_TOKEN_ACCOUNT;
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
    fn pumpfun_v2_sell_fixed_output_uses_min_sol_directly() {
        let mint = pump_mint();
        let mut params = swap_params_for_buy(mint, TOKEN_PROGRAM);
        params.trade_type = crate::swqos::TradeType::Sell;
        params.input_mint = mint;
        params.output_mint = crate::constants::SOL_TOKEN_ACCOUNT;
        params.create_output_mint_ata = false;
        params.fixed_output_amount = Some(42);
        if let DexParamEnum::PumpFun(protocol_params) = &mut params.protocol_params {
            *protocol_params =
                protocol_params.clone().with_quote_mint(crate::constants::WSOL_TOKEN_ACCOUNT);
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
