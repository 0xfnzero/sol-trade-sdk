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
        utils::meteora_dlmm::{EVENT_AUTHORITY, MEMO_PROGRAM, PROGRAM_ID, SWAP2_DISCRIMINATOR},
    },
    trading::core::{
        params::{MeteoraDlmmParams, SwapParams},
        traits::InstructionBuilder,
    },
};

#[derive(Clone, Debug)]
pub struct MeteoraDlmmSwap2Accounts {
    pub lb_pair: Pubkey,
    pub bitmap_extension: Option<Pubkey>,
    pub reserve_x: Pubkey,
    pub reserve_y: Pubkey,
    pub user_token_in: Pubkey,
    pub user_token_out: Pubkey,
    pub token_x_mint: Pubkey,
    pub token_y_mint: Pubkey,
    pub oracle: Pubkey,
    pub user: Pubkey,
    pub token_x_program: Pubkey,
    pub token_y_program: Pubkey,
    pub bin_arrays: Vec<Pubkey>,
}

pub fn swap2(
    accounts: &MeteoraDlmmSwap2Accounts,
    amount_in: u64,
    min_out: u64,
) -> Result<Instruction> {
    if accounts.bin_arrays.is_empty() {
        return Err(anyhow!("Meteora DLMM swap2 requires at least one bin array"));
    }
    let mut metas = Vec::with_capacity(16 + accounts.bin_arrays.len());
    let bitmap_meta = match accounts.bitmap_extension {
        Some(key) => AccountMeta::new(key, false),
        None => AccountMeta::new_readonly(PROGRAM_ID, false),
    };
    metas.extend([
        AccountMeta::new(accounts.lb_pair, false),
        bitmap_meta,
        AccountMeta::new(accounts.reserve_x, false),
        AccountMeta::new(accounts.reserve_y, false),
        AccountMeta::new(accounts.user_token_in, false),
        AccountMeta::new(accounts.user_token_out, false),
        AccountMeta::new_readonly(accounts.token_x_mint, false),
        AccountMeta::new_readonly(accounts.token_y_mint, false),
        AccountMeta::new(accounts.oracle, false),
        AccountMeta::new_readonly(PROGRAM_ID, false), // host_fee_in None sentinel
        AccountMeta::new_readonly(accounts.user, true),
        AccountMeta::new_readonly(accounts.token_x_program, false),
        AccountMeta::new_readonly(accounts.token_y_program, false),
        AccountMeta::new_readonly(MEMO_PROGRAM, false),
        AccountMeta::new_readonly(EVENT_AUTHORITY, false),
        AccountMeta::new_readonly(PROGRAM_ID, false),
    ]);
    metas.extend(accounts.bin_arrays.iter().map(|key| AccountMeta::new(*key, false)));
    // IDL swap2 args: amount_in + min_amount_out + RemainingAccountsInfo { slices: Vec }
    // Empty slices → Borsh u32 length 0 (4 zero bytes after min_out).
    let mut data = [0u8; 28];
    data[..8].copy_from_slice(&SWAP2_DISCRIMINATOR);
    data[8..16].copy_from_slice(&amount_in.to_le_bytes());
    data[16..24].copy_from_slice(&min_out.to_le_bytes());
    Ok(Instruction::new_with_bytes(PROGRAM_ID, &data, metas))
}

/// Instruction builder for Meteora DLMM `swap2` (exact-in).
pub struct MeteoraDlmmInstructionBuilder;

async fn build_dlmm_swap(params: &SwapParams) -> Result<Vec<Instruction>> {
    let amount_in = params.input_amount.unwrap_or(0);
    if amount_in == 0 {
        return Err(anyhow!("Amount cannot be zero"));
    }
    let min_out = params.fixed_output_amount.ok_or_else(|| {
        anyhow!("MeteoraDlmm requires fixed_output_amount (min out / quoted threshold)")
    })?;
    let protocol = params
        .protocol_params
        .as_any()
        .downcast_ref::<MeteoraDlmmParams>()
        .ok_or_else(|| anyhow!("Invalid protocol params for MeteoraDlmm"))?;
    if protocol.bin_arrays.is_empty() {
        return Err(anyhow!("Meteora DLMM requires bin arrays"));
    }

    let (input_program, output_program) =
        if params.input_mint == protocol.token_x_mint && params.output_mint == protocol.token_y_mint
        {
            (protocol.token_x_program, protocol.token_y_program)
        } else if params.input_mint == protocol.token_y_mint
            && params.output_mint == protocol.token_x_mint
        {
            (protocol.token_y_program, protocol.token_x_program)
        } else {
            return Err(anyhow!("Meteora DLMM swap pair does not match pool mints"));
        };

    let payer = params.payer.pubkey();
    let user_in = get_associated_token_address_with_program_id_fast_use_seed(
        &payer,
        &params.input_mint,
        &input_program,
        params.open_seed_optimize,
    );
    let user_out = get_associated_token_address_with_program_id_fast_use_seed(
        &payer,
        &params.output_mint,
        &output_program,
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

    instructions.push(swap2(
        &MeteoraDlmmSwap2Accounts {
            lb_pair: protocol.lb_pair,
            bitmap_extension: protocol.bitmap_extension,
            reserve_x: protocol.reserve_x,
            reserve_y: protocol.reserve_y,
            user_token_in: user_in,
            user_token_out: user_out,
            token_x_mint: protocol.token_x_mint,
            token_y_mint: protocol.token_y_mint,
            oracle: protocol.oracle,
            user: payer,
            token_x_program: protocol.token_x_program,
            token_y_program: protocol.token_y_program,
            bin_arrays: protocol.bin_arrays.clone(),
        },
        amount_in,
        min_out,
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
impl InstructionBuilder for MeteoraDlmmInstructionBuilder {
    async fn build_buy_instructions(&self, params: &SwapParams) -> Result<Vec<Instruction>> {
        build_dlmm_swap(params).await
    }

    async fn build_sell_instructions(&self, params: &SwapParams) -> Result<Vec<Instruction>> {
        build_dlmm_swap(params).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instruction::utils::meteora_dlmm::{PROGRAM_ID, SWAP2_DISCRIMINATOR};
    use solana_sdk::pubkey::Pubkey;

    #[test]
    fn swap2_encodes_empty_remaining_accounts_info() {
        let bin = Pubkey::new_unique();
        let ix = swap2(
            &MeteoraDlmmSwap2Accounts {
                lb_pair: Pubkey::new_unique(),
                bitmap_extension: None,
                reserve_x: Pubkey::new_unique(),
                reserve_y: Pubkey::new_unique(),
                user_token_in: Pubkey::new_unique(),
                user_token_out: Pubkey::new_unique(),
                token_x_mint: Pubkey::new_unique(),
                token_y_mint: Pubkey::new_unique(),
                oracle: Pubkey::new_unique(),
                user: Pubkey::new_unique(),
                token_x_program: Pubkey::new_unique(),
                token_y_program: Pubkey::new_unique(),
                bin_arrays: vec![bin],
            },
            1_000,
            900,
        )
        .unwrap();
        assert_eq!(ix.data.len(), 28);
        assert_eq!(&ix.data[..8], &SWAP2_DISCRIMINATOR);
        assert_eq!(u64::from_le_bytes(ix.data[8..16].try_into().unwrap()), 1_000);
        assert_eq!(u64::from_le_bytes(ix.data[16..24].try_into().unwrap()), 900);
        assert_eq!(&ix.data[24..28], &[0, 0, 0, 0]);
        // bitmap missing → program-id sentinel (readonly)
        assert!(!ix.accounts[1].is_writable);
        assert_eq!(ix.accounts[1].pubkey, PROGRAM_ID);
        assert!(ix.accounts.last().unwrap().is_writable);
        assert_eq!(ix.accounts.last().unwrap().pubkey, bin);
    }
}
