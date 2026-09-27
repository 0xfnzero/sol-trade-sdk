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
        utils::whirlpool::{
            default_sqrt_price_limit, oracle, MEMO_PROGRAM, PROGRAM_ID, SWAP_V2_DISCRIMINATOR,
        },
    },
    trading::core::{
        params::{SwapParams, WhirlpoolParams},
        traits::InstructionBuilder,
    },
};

#[derive(Clone, Debug)]
pub struct WhirlpoolSwapV2Accounts {
    pub token_program_a: Pubkey,
    pub token_program_b: Pubkey,
    pub token_authority: Pubkey,
    pub whirlpool: Pubkey,
    pub mint_a: Pubkey,
    pub mint_b: Pubkey,
    pub owner_a: Pubkey,
    pub vault_a: Pubkey,
    pub owner_b: Pubkey,
    pub vault_b: Pubkey,
    pub tick_arrays: Vec<Pubkey>,
}

#[derive(Clone, Copy, Debug)]
pub struct WhirlpoolSwapV2Args {
    pub amount: u64,
    pub other_amount_threshold: u64,
    /// Pass `0` to use explicit full-range defaults (`MIN`/`MAX`). On-chain also
    /// treats `0` as no-limit and remaps the same way.
    pub sqrt_price_limit: u128,
    pub amount_specified_is_input: bool,
    pub a_to_b: bool,
}

pub fn swap_v2(
    accounts: &WhirlpoolSwapV2Accounts,
    args: WhirlpoolSwapV2Args,
) -> Result<Instruction> {
    if accounts.tick_arrays.len() < 3 {
        return Err(anyhow!(
            "Whirlpool swap_v2 requires 3 tick arrays (got {})",
            accounts.tick_arrays.len()
        ));
    }
    let ticks = [
        accounts.tick_arrays[0],
        accounts.tick_arrays[1],
        accounts.tick_arrays[2],
    ];
    let metas = vec![
        AccountMeta::new_readonly(accounts.token_program_a, false),
        AccountMeta::new_readonly(accounts.token_program_b, false),
        AccountMeta::new_readonly(MEMO_PROGRAM, false),
        AccountMeta::new_readonly(accounts.token_authority, true),
        AccountMeta::new(accounts.whirlpool, false),
        AccountMeta::new_readonly(accounts.mint_a, false),
        AccountMeta::new_readonly(accounts.mint_b, false),
        AccountMeta::new(accounts.owner_a, false),
        AccountMeta::new(accounts.vault_a, false),
        AccountMeta::new(accounts.owner_b, false),
        AccountMeta::new(accounts.vault_b, false),
        AccountMeta::new(ticks[0], false),
        AccountMeta::new(ticks[1], false),
        AccountMeta::new(ticks[2], false),
        AccountMeta::new(oracle(&accounts.whirlpool), false),
    ];
    let sqrt_price_limit = if args.sqrt_price_limit == 0 {
        default_sqrt_price_limit(args.a_to_b)
    } else {
        args.sqrt_price_limit
    };
    let mut data = Vec::with_capacity(43);
    data.extend_from_slice(&SWAP_V2_DISCRIMINATOR);
    data.extend_from_slice(&args.amount.to_le_bytes());
    data.extend_from_slice(&args.other_amount_threshold.to_le_bytes());
    data.extend_from_slice(&sqrt_price_limit.to_le_bytes());
    data.push(u8::from(args.amount_specified_is_input));
    data.push(u8::from(args.a_to_b));
    data.push(0); // remaining_accounts_info: None
    Ok(Instruction::new_with_bytes(PROGRAM_ID, &data, metas))
}

/// Instruction builder for Orca Whirlpool `swap_v2` (exact-in).
pub struct WhirlpoolInstructionBuilder;

async fn build_whirlpool_swap(params: &SwapParams) -> Result<Vec<Instruction>> {
    let amount_in = params.input_amount.unwrap_or(0);
    if amount_in == 0 {
        return Err(anyhow!("Amount cannot be zero"));
    }
    let min_out = params.fixed_output_amount.ok_or_else(|| {
        anyhow!("Whirlpool requires fixed_output_amount (min out / quoted threshold)")
    })?;
    let protocol = params
        .protocol_params
        .as_any()
        .downcast_ref::<WhirlpoolParams>()
        .ok_or_else(|| anyhow!("Invalid protocol params for Whirlpool"))?;
    if protocol.tick_arrays.is_empty() {
        return Err(anyhow!("Whirlpool requires tick arrays"));
    }

    let a_to_b = if params.input_mint == protocol.mint_a && params.output_mint == protocol.mint_b {
        true
    } else if params.input_mint == protocol.mint_b && params.output_mint == protocol.mint_a {
        false
    } else {
        return Err(anyhow!("Whirlpool swap pair does not match pool mints"));
    };

    let (input_program, output_program) = if a_to_b {
        (protocol.token_program_a, protocol.token_program_b)
    } else {
        (protocol.token_program_b, protocol.token_program_a)
    };
    let payer = params.payer.pubkey();
    let owner_a = get_associated_token_address_with_program_id_fast_use_seed(
        &payer,
        &protocol.mint_a,
        &protocol.token_program_a,
        params.open_seed_optimize,
    );
    let owner_b = get_associated_token_address_with_program_id_fast_use_seed(
        &payer,
        &protocol.mint_b,
        &protocol.token_program_b,
        params.open_seed_optimize,
    );

    let mut instructions = Vec::with_capacity(6);
    if params.create_input_mint_ata {
        push_create_or_wrap_user_token_account(
            &mut instructions,
            &payer,
            &params.input_mint,
            &input_program,
            amount_in,
            params.open_seed_optimize,
        );
    }
    if params.create_output_mint_ata {
        push_create_user_token_account(
            &mut instructions,
            &payer,
            &params.output_mint,
            &output_program,
            params.open_seed_optimize,
        );
    }

    instructions.push(swap_v2(
        &WhirlpoolSwapV2Accounts {
            token_program_a: protocol.token_program_a,
            token_program_b: protocol.token_program_b,
            token_authority: payer,
            whirlpool: protocol.whirlpool,
            mint_a: protocol.mint_a,
            mint_b: protocol.mint_b,
            owner_a,
            vault_a: protocol.vault_a,
            owner_b,
            vault_b: protocol.vault_b,
            tick_arrays: protocol.tick_arrays.clone(),
        },
        WhirlpoolSwapV2Args {
            amount: amount_in,
            other_amount_threshold: min_out,
            sqrt_price_limit: protocol.sqrt_price_limit,
            amount_specified_is_input: true,
            a_to_b,
        },
    )?);

    if params.close_input_mint_ata {
        push_close_wsol_if_needed(&mut instructions, &payer, &params.input_mint);
    }
    if params.close_output_mint_ata {
        push_close_wsol_if_needed(&mut instructions, &payer, &params.output_mint);
    }
    Ok(instructions)
}

#[async_trait::async_trait]
impl InstructionBuilder for WhirlpoolInstructionBuilder {
    async fn build_buy_instructions(&self, params: &SwapParams) -> Result<Vec<Instruction>> {
        build_whirlpool_swap(params).await
    }

    async fn build_sell_instructions(&self, params: &SwapParams) -> Result<Vec<Instruction>> {
        build_whirlpool_swap(params).await
    }
}
