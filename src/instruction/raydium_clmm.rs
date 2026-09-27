use anyhow::{anyhow, Result};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signer::Signer,
};

use crate::{
    common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed,
    instruction::{
        token_account_setup::{
            push_close_wsol_if_needed, push_create_or_wrap_user_token_account,
            push_create_user_token_account,
        },
        utils::raydium_clmm::{
            tick_array_bitmap_extension, MAX_SQRT_PRICE_X64, MEMO_PROGRAM, MIN_SQRT_PRICE_X64,
            PROGRAM_ID, SWAP_V2_DISCRIMINATOR,
        },
    },
    trading::core::{
        params::{RaydiumClmmParams, SwapParams},
        traits::InstructionBuilder,
    },
};

#[derive(Clone, Debug)]
pub struct RaydiumClmmSwapV2Accounts {
    pub payer: Pubkey,
    pub amm_config: Pubkey,
    pub pool_state: Pubkey,
    pub input_token_account: Pubkey,
    pub output_token_account: Pubkey,
    pub input_vault: Pubkey,
    pub output_vault: Pubkey,
    pub observation_state: Pubkey,
    pub token_program: Pubkey,
    pub token_program_2022: Pubkey,
    pub input_vault_mint: Pubkey,
    pub output_vault_mint: Pubkey,
    pub tick_array_bitmap_extension: Option<Pubkey>,
    pub tick_arrays: Vec<Pubkey>,
}

#[derive(Clone, Copy, Debug)]
pub struct RaydiumClmmSwapV2Args {
    pub amount: u64,
    pub other_amount_threshold: u64,
    pub sqrt_price_limit_x64: u128,
    pub is_base_input: bool,
}

pub fn swap_v2(
    accounts: &RaydiumClmmSwapV2Accounts,
    args: RaydiumClmmSwapV2Args,
) -> Result<Instruction> {
    let bitmap_pda = tick_array_bitmap_extension(&accounts.pool_state);
    // Align with on-chain remaining-account scan: bitmap may be mixed into tick list.
    let mut bitmap = accounts
        .tick_array_bitmap_extension
        .filter(|b| *b == bitmap_pda);
    let mut tick_arrays = Vec::with_capacity(accounts.tick_arrays.len());
    for &key in &accounts.tick_arrays {
        if key == bitmap_pda {
            bitmap = Some(key);
        } else {
            tick_arrays.push(key);
        }
    }
    if tick_arrays.is_empty() {
        return Err(anyhow!("Raydium CLMM swap_v2 requires at least one tick array"));
    }
    let mut metas = Vec::with_capacity(13 + tick_arrays.len() + usize::from(bitmap.is_some()));
    metas.extend([
        AccountMeta::new_readonly(accounts.payer, true),
        AccountMeta::new_readonly(accounts.amm_config, false),
        AccountMeta::new(accounts.pool_state, false),
        AccountMeta::new(accounts.input_token_account, false),
        AccountMeta::new(accounts.output_token_account, false),
        AccountMeta::new(accounts.input_vault, false),
        AccountMeta::new(accounts.output_vault, false),
        AccountMeta::new(accounts.observation_state, false),
        AccountMeta::new_readonly(accounts.token_program, false),
        AccountMeta::new_readonly(accounts.token_program_2022, false),
        AccountMeta::new_readonly(MEMO_PROGRAM, false),
        AccountMeta::new_readonly(accounts.input_vault_mint, false),
        AccountMeta::new_readonly(accounts.output_vault_mint, false),
    ]);
    if let Some(ext) = bitmap {
        metas.push(AccountMeta::new(ext, false));
    }
    metas.extend(tick_arrays.iter().map(|key| AccountMeta::new(*key, false)));
    let mut data = [0u8; 41];
    data[..8].copy_from_slice(&SWAP_V2_DISCRIMINATOR);
    data[8..16].copy_from_slice(&args.amount.to_le_bytes());
    data[16..24].copy_from_slice(&args.other_amount_threshold.to_le_bytes());
    data[24..40].copy_from_slice(&args.sqrt_price_limit_x64.to_le_bytes());
    data[40] = u8::from(args.is_base_input);
    Ok(Instruction::new_with_bytes(PROGRAM_ID, &data, metas))
}

/// Instruction builder for Raydium CLMM `swap_v2` (exact-in).
pub struct RaydiumClmmInstructionBuilder;

fn clmm_sqrt_limit(zero_for_one: bool, explicit: u128) -> u128 {
    if explicit != 0 {
        return explicit;
    }
    if zero_for_one {
        MIN_SQRT_PRICE_X64 + 1
    } else {
        MAX_SQRT_PRICE_X64 - 1
    }
}

async fn build_clmm_swap(params: &SwapParams) -> Result<Vec<Instruction>> {
    let amount_in = params.input_amount.unwrap_or(0);
    if amount_in == 0 {
        return Err(anyhow!("Amount cannot be zero"));
    }
    let min_out = params.fixed_output_amount.ok_or_else(|| {
        anyhow!("RaydiumClmm requires fixed_output_amount (min out / quoted threshold)")
    })?;
    let protocol = params
        .protocol_params
        .as_any()
        .downcast_ref::<RaydiumClmmParams>()
        .ok_or_else(|| anyhow!("Invalid protocol params for RaydiumClmm"))?;
    if protocol.tick_arrays.is_empty() {
        return Err(anyhow!("Raydium CLMM requires tick arrays"));
    }

    let input_mint = params.input_mint;
    let output_mint = params.output_mint;
    let (input_vault, output_vault, input_program, output_program, zero_for_one) =
        if input_mint == protocol.token_0_mint && output_mint == protocol.token_1_mint {
            (
                protocol.token_0_vault,
                protocol.token_1_vault,
                protocol.token_0_program,
                protocol.token_1_program,
                true,
            )
        } else if input_mint == protocol.token_1_mint && output_mint == protocol.token_0_mint {
            (
                protocol.token_1_vault,
                protocol.token_0_vault,
                protocol.token_1_program,
                protocol.token_0_program,
                false,
            )
        } else {
            return Err(anyhow!("Raydium CLMM swap pair does not match pool mints"));
        };

    let payer = params.payer.pubkey();
    let input_ata = get_associated_token_address_with_program_id_fast_use_seed(
        &payer,
        &input_mint,
        &input_program,
        params.open_seed_optimize,
    );
    let output_ata = get_associated_token_address_with_program_id_fast_use_seed(
        &payer,
        &output_mint,
        &output_program,
        params.open_seed_optimize,
    );

    let mut instructions = Vec::with_capacity(6);
    if params.create_input_mint_ata {
        push_create_or_wrap_user_token_account(
            &mut instructions,
            &payer,
            &input_mint,
            &input_program,
            amount_in,
            params.open_seed_optimize,
        );
    }
    if params.create_output_mint_ata {
        push_create_user_token_account(
            &mut instructions,
            &payer,
            &output_mint,
            &output_program,
            params.open_seed_optimize,
        );
    }

    instructions.push(swap_v2(
        &RaydiumClmmSwapV2Accounts {
            payer,
            amm_config: protocol.amm_config,
            pool_state: protocol.pool_state,
            input_token_account: input_ata,
            output_token_account: output_ata,
            input_vault,
            output_vault,
            observation_state: protocol.observation_state,
            token_program: crate::constants::TOKEN_PROGRAM,
            token_program_2022: crate::constants::TOKEN_PROGRAM_2022,
            input_vault_mint: input_mint,
            output_vault_mint: output_mint,
            tick_array_bitmap_extension: protocol.tick_array_bitmap_extension,
            tick_arrays: protocol.tick_arrays.clone(),
        },
        RaydiumClmmSwapV2Args {
            amount: amount_in,
            other_amount_threshold: min_out,
            sqrt_price_limit_x64: clmm_sqrt_limit(zero_for_one, protocol.sqrt_price_limit_x64),
            is_base_input: true,
        },
    )?);

    if params.close_input_mint_ata {
        push_close_wsol_if_needed(&mut instructions, &payer, &input_mint);
    }
    if params.close_output_mint_ata {
        push_close_wsol_if_needed(&mut instructions, &payer, &output_mint);
    }
    Ok(instructions)
}

#[async_trait::async_trait]
impl InstructionBuilder for RaydiumClmmInstructionBuilder {
    async fn build_buy_instructions(&self, params: &SwapParams) -> Result<Vec<Instruction>> {
        build_clmm_swap(params).await
    }

    async fn build_sell_instructions(&self, params: &SwapParams) -> Result<Vec<Instruction>> {
        build_clmm_swap(params).await
    }
}
