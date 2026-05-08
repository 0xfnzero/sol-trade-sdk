//! Pump.fun bonding-curve swap ix assembly ([`SwapParams`](crate::trading::core::params::SwapParams)).

use crate::{
    common::spl_token::close_account,
    constants::{trade::trade::DEFAULT_SLIPPAGE, TOKEN_PROGRAM_2022},
    trading::core::{
        params::{PumpFunParams, SwapParams},
        traits::InstructionBuilder,
    },
};
use crate::{
    instruction::pumpfun_ix_data::{
        encode_pumpfun_buy_exact_quote_in_v2_ix_data, encode_pumpfun_buy_exact_sol_in_ix_data,
        encode_pumpfun_buy_ix_data, encode_pumpfun_buy_v2_ix_data,
        encode_pumpfun_sell_ix_data, encode_pumpfun_sell_v2_ix_data, TRACK_VOLUME_TRUE,
    },
    instruction::utils::pumpfun::{
        accounts, get_bonding_curve_pda, get_bonding_curve_v2_pda,
        get_buyback_fee_recipient_random, get_protocol_extra_fee_recipient_random,
        get_quote_token_program, get_user_volume_accumulator_pda,
        pump_fun_fee_recipient_meta, resolve_creator_vault_for_ix_with_fee_sharing,
        global_constants::{self},
    },
    utils::calc::{
        common::{calculate_with_slippage_buy, calculate_with_slippage_sell},
        pumpfun::{get_buy_token_amount_from_sol_amount, get_sell_sol_amount_from_token_amount},
    },
};
use anyhow::{anyhow, Result};
use solana_sdk::instruction::AccountMeta;
use solana_sdk::{instruction::Instruction, pubkey::Pubkey, signer::Signer};

#[inline]
fn effective_pump_mint_token_program(protocol_params: &PumpFunParams) -> Pubkey {
    let tp = protocol_params.token_program;
    if tp == Pubkey::default() {
        crate::constants::TOKEN_PROGRAM // legacy SPL Token — safe default; Token-2022 must be explicit
    } else {
        tp
    }
}

pub struct PumpFunInstructionBuilder;

#[async_trait::async_trait]
impl InstructionBuilder for PumpFunInstructionBuilder {
    async fn build_buy_instructions(&self, params: &SwapParams) -> Result<Vec<Instruction>> {
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
        .ok_or_else(|| {
            anyhow!(
                "creator_vault PDA derivation failed (creator={})",
                creator
            )
        })?;

        let buy_token_amount = match params.fixed_output_amount {
            Some(amount) => amount,
            None => get_buy_token_amount_from_sol_amount(
                bonding_curve.virtual_token_reserves as u128,
                bonding_curve.virtual_sol_reserves as u128,
                bonding_curve.real_token_reserves as u128,
                creator,
                lamports_in,
            ),
        };

        let max_sol_cost = calculate_with_slippage_buy(lamports_in, slippage_bp);

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

        let fee_recipient_meta =
            pump_fun_fee_recipient_meta(protocol_params.fee_recipient, is_mayhem_mode);

        // ── V2 instruction path (buy_v2 / buy_exact_quote_in_v2, 27 fixed accounts) ──
        if protocol_params.use_v2_ix {
            let quote_mint = if protocol_params.quote_mint != Pubkey::default() {
                protocol_params.quote_mint
            } else {
                crate::constants::WSOL_TOKEN_ACCOUNT
            };
            let quote_token_program = get_quote_token_program(&quote_mint);
            let quote_token_program_meta = if quote_token_program == crate::constants::TOKEN_PROGRAM {
                crate::constants::TOKEN_PROGRAM_META
            } else {
                crate::constants::TOKEN_PROGRAM_2022_META
            };

            let associated_quote_bonding_curve =
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &bonding_curve_addr, &quote_mint, &quote_token_program,
                );
            let associated_quote_user =
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                    &params.payer.pubkey(), &quote_mint, &quote_token_program,
                    params.open_seed_optimize,
                );
            let associated_quote_fee_recipient =
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &fee_recipient_meta.pubkey, &quote_mint, &quote_token_program,
                );
            let buyback_fee_recipient =
                get_buyback_fee_recipient_random();
            let associated_quote_buyback_fee_recipient =
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &buyback_fee_recipient, &quote_mint, &quote_token_program,
                );
            let associated_creator_vault =
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &creator_vault_account, &quote_mint, &quote_token_program,
                );
            let associated_user_volume_accumulator =
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &user_volume_accumulator, &quote_mint, &quote_token_program,
                );
            let sharing_config =
                crate::instruction::utils::pumpfun::get_fee_sharing_config_pda(&params.output_mint)
                    .ok_or_else(|| anyhow!("sharing_config PDA derivation failed"))?;

            let v2_data = if params.use_exact_sol_amount.unwrap_or(true) {
                let min_tokens_out = calculate_with_slippage_sell(buy_token_amount, slippage_bp);
                encode_pumpfun_buy_exact_quote_in_v2_ix_data(lamports_in, min_tokens_out)
            } else {
                encode_pumpfun_buy_v2_ix_data(buy_token_amount, max_sol_cost)
            };

            let v2_metas: Vec<AccountMeta> = vec![
                global_constants::GLOBAL_ACCOUNT_META,
                AccountMeta::new_readonly(params.output_mint, false),
                AccountMeta::new_readonly(quote_mint, false),
                token_program_meta,
                quote_token_program_meta,
                crate::constants::ASSOCIATED_TOKEN_PROGRAM_META,
                fee_recipient_meta,
                AccountMeta::new(associated_quote_fee_recipient, false),
                AccountMeta::new_readonly(buyback_fee_recipient, false),
                AccountMeta::new(associated_quote_buyback_fee_recipient, false),
                AccountMeta::new(bonding_curve_addr, false),
                AccountMeta::new(associated_bonding_curve, false),
                AccountMeta::new(associated_quote_bonding_curve, false),
                AccountMeta::new(params.payer.pubkey(), true),
                AccountMeta::new(user_token_account, false),
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

            if quote_mint != crate::constants::WSOL_TOKEN_ACCOUNT {
                let create_quote_ata = crate::common::fast_fn::create_associated_token_account_idempotent_fast_use_seed(
                    &params.payer.pubkey(), &params.payer.pubkey(),
                    &quote_mint, &quote_token_program, params.open_seed_optimize,
                );
                if !create_quote_ata.is_empty() {
                    instructions.extend(create_quote_ata);
                }
            }

            instructions.push(Instruction::new_with_bytes(accounts::PUMPFUN, &v2_data, v2_metas));
            return Ok(instructions);
        }

        // ── Legacy buy path ──
        let buy_data = if params.use_exact_sol_amount.unwrap_or(true) {
            let min_tokens_out = calculate_with_slippage_sell(buy_token_amount, slippage_bp);
            encode_pumpfun_buy_exact_sol_in_ix_data(
                lamports_in,
                min_tokens_out,
                TRACK_VOLUME_TRUE,
            )
        } else {
            encode_pumpfun_buy_ix_data(buy_token_amount, max_sol_cost, TRACK_VOLUME_TRUE)
        };

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
        metas.push(AccountMeta::new(
            get_protocol_extra_fee_recipient_random(),
            false,
        ));

        instructions.push(Instruction::new_with_bytes(
            accounts::PUMPFUN,
            &buy_data,
            metas,
        ));

        Ok(instructions)
    }

    async fn build_sell_instructions(&self, params: &SwapParams) -> Result<Vec<Instruction>> {
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
        let creator = protocol_params.effective_creator_for_trade();
        let creator_vault_account = resolve_creator_vault_for_ix_with_fee_sharing(
            &creator,
            protocol_params.creator_vault,
            &params.input_mint,
            protocol_params.fee_sharing_creator_vault_if_active,
        )
        .ok_or_else(|| {
            anyhow!(
                "creator_vault PDA derivation failed (creator={})",
                creator
            )
        })?;

        let sol_amount = get_sell_sol_amount_from_token_amount(
            bonding_curve.virtual_token_reserves as u128,
            bonding_curve.virtual_sol_reserves as u128,
            creator,
            token_amount,
        );

        let min_sol_output = match params.fixed_output_amount {
            Some(fixed) => fixed,
            None => calculate_with_slippage_sell(sol_amount, slippage_bp),
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

        let mut instructions = Vec::with_capacity(2);

        // ── V2 sell path (sell_v2, 26 fixed accounts) ──
        if protocol_params.use_v2_ix {
            let quote_mint = if protocol_params.quote_mint != Pubkey::default() {
                protocol_params.quote_mint
            } else {
                crate::constants::WSOL_TOKEN_ACCOUNT
            };
            let quote_token_program = get_quote_token_program(&quote_mint);
            let quote_token_program_meta = if quote_token_program == crate::constants::TOKEN_PROGRAM {
                crate::constants::TOKEN_PROGRAM_META
            } else {
                crate::constants::TOKEN_PROGRAM_2022_META
            };

            let associated_quote_bonding_curve =
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &bonding_curve_addr, &quote_mint, &quote_token_program,
                );
            let associated_quote_user =
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                    &params.payer.pubkey(), &quote_mint, &quote_token_program,
                    params.open_seed_optimize,
                );

            let sell_fee_recipient_meta =
                pump_fun_fee_recipient_meta(protocol_params.fee_recipient, is_mayhem_mode);
            let associated_quote_fee_recipient =
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &sell_fee_recipient_meta.pubkey, &quote_mint, &quote_token_program,
                );
            let buyback_fee_recipient =
                get_buyback_fee_recipient_random();
            let associated_quote_buyback_fee_recipient =
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &buyback_fee_recipient, &quote_mint, &quote_token_program,
                );
            let associated_creator_vault =
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &creator_vault_account, &quote_mint, &quote_token_program,
                );
            let user_volume_accumulator = get_user_volume_accumulator_pda(&params.payer.pubkey())
                .ok_or_else(|| anyhow!("user_volume_accumulator PDA derivation failed"))?;
            let associated_user_volume_accumulator =
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &user_volume_accumulator, &quote_mint, &quote_token_program,
                );
            let sharing_config =
                crate::instruction::utils::pumpfun::get_fee_sharing_config_pda(&params.input_mint)
                    .ok_or_else(|| anyhow!("sharing_config PDA derivation failed"))?;

            let v2_sell_data = encode_pumpfun_sell_v2_ix_data(token_amount, min_sol_output);

            let v2_sell_metas: Vec<AccountMeta> = vec![
                global_constants::GLOBAL_ACCOUNT_META,
                AccountMeta::new_readonly(params.input_mint, false),
                AccountMeta::new_readonly(quote_mint, false),
                token_program_meta,
                quote_token_program_meta,
                crate::constants::ASSOCIATED_TOKEN_PROGRAM_META,
                sell_fee_recipient_meta,
                AccountMeta::new(associated_quote_fee_recipient, false),
                AccountMeta::new_readonly(buyback_fee_recipient, false),
                AccountMeta::new(associated_quote_buyback_fee_recipient, false),
                AccountMeta::new(bonding_curve_addr, false),
                AccountMeta::new(associated_bonding_curve, false),
                AccountMeta::new(associated_quote_bonding_curve, false),
                AccountMeta::new(params.payer.pubkey(), true),
                AccountMeta::new(user_token_account, false),
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

            instructions.push(Instruction::new_with_bytes(accounts::PUMPFUN, &v2_sell_data, v2_sell_metas));

            if protocol_params.close_token_account_when_sell.unwrap_or(false)
                || params.close_input_mint_ata
            {
                instructions.push(close_account(
                    &token_program, &user_token_account,
                    &params.payer.pubkey(), &params.payer.pubkey(),
                    &[&params.payer.pubkey()],
                )?);
            }

            return Ok(instructions);
        }

        // ── Legacy sell path ──
        let sell_data = encode_pumpfun_sell_ix_data(token_amount, min_sol_output);
        let fee_recipient_meta =
            pump_fun_fee_recipient_meta(protocol_params.fee_recipient, is_mayhem_mode);

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
            let user_volume_accumulator =
                get_user_volume_accumulator_pda(&params.payer.pubkey())
                    .ok_or_else(|| anyhow!("user_volume_accumulator PDA derivation failed"))?;
            metas.push(AccountMeta::new(user_volume_accumulator, false));
        }

        let bonding_curve_v2 = get_bonding_curve_v2_pda(&params.input_mint).ok_or_else(|| {
            anyhow!("bonding_curve_v2 PDA derivation failed for mint {}", params.input_mint)
        })?;
        metas.push(AccountMeta::new_readonly(bonding_curve_v2, false));
        metas.push(AccountMeta::new(
            get_protocol_extra_fee_recipient_random(),
            false,
        ));

        instructions.push(Instruction::new_with_bytes(
            accounts::PUMPFUN,
            &sell_data,
            metas,
        ));

        if protocol_params.close_token_account_when_sell.unwrap_or(false)
            || params.close_input_mint_ata
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
}

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
    Some(Instruction::new_with_bytes(
        accounts::PUMPFUN,
        &CLAIM_CASHBACK_DISCRIMINATOR,
        ix_accounts,
    ))
}
