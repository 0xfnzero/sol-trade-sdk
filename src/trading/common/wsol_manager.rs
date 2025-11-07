use crate::common::{
    fast_fn::create_associated_token_account_idempotent_fast, spl_token::close_account,
};
use smallvec::SmallVec;
use solana_sdk::{instruction::Instruction, message::AccountMeta, pubkey::Pubkey};
use solana_system_interface::instruction::transfer;

#[inline]
pub fn handle_wsol(payer: &Pubkey, amount_in: u64) -> SmallVec<[Instruction; 3]> {
    let wsol_token_account =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &payer,
            &crate::constants::WSOL_TOKEN_ACCOUNT,
            &crate::constants::TOKEN_PROGRAM,
        );

    let mut insts = SmallVec::<[Instruction; 3]>::new();
    insts.extend(create_associated_token_account_idempotent_fast(
        &payer,
        &payer,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
        &crate::constants::TOKEN_PROGRAM,
    ));
    insts.extend([
        transfer(&payer, &wsol_token_account, amount_in),
        // sync_native
        Instruction {
            program_id: crate::constants::TOKEN_PROGRAM,
            accounts: vec![AccountMeta::new(wsol_token_account, false)],
            data: vec![17],
        },
    ]);

    insts
}

pub fn close_wsol(payer: &Pubkey) -> Vec<Instruction> {
    let wsol_token_account =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &payer,
            &crate::constants::WSOL_TOKEN_ACCOUNT,
            &crate::constants::TOKEN_PROGRAM,
        );
    crate::common::fast_fn::get_cached_instructions(
        crate::common::fast_fn::InstructionCacheKey::CloseWsolAccount {
            payer: *payer,
            wsol_token_account,
        },
        || {
            vec![close_account(
                &crate::constants::TOKEN_PROGRAM,
                &wsol_token_account,
                &payer,
                &payer,
                &[],
            )
            .unwrap()]
        },
    )
}

#[inline]
pub fn create_wsol_ata(payer: &Pubkey) -> Vec<Instruction> {
    create_associated_token_account_idempotent_fast(
        &payer,
        &payer,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
        &crate::constants::TOKEN_PROGRAM,
    )
}

/// 只充值SOL到已存在的WSOL ATA（不创建账户）- 标准方式
#[inline]
pub fn wrap_sol_only(payer: &Pubkey, amount_in: u64) -> SmallVec<[Instruction; 2]> {
    let wsol_token_account =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &payer,
            &crate::constants::WSOL_TOKEN_ACCOUNT,
            &crate::constants::TOKEN_PROGRAM,
        );

    let mut insts = SmallVec::<[Instruction; 2]>::new();
    insts.extend([
        transfer(&payer, &wsol_token_account, amount_in),
        // sync_native
        Instruction {
            program_id: crate::constants::TOKEN_PROGRAM,
            accounts: vec![AccountMeta::new(wsol_token_account, false)],
            data: vec![17],
        },
    ]);

    insts
}
