use crate::common::fast_fn::create_associated_token_account_idempotent_fast;
use smallvec::SmallVec;
use solana_sdk::{instruction::Instruction, pubkey::Pubkey};
use solana_system_interface::instruction::transfer;
use spl_token::instruction::close_account;

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
        spl_token::instruction::sync_native(&crate::constants::TOKEN_PROGRAM, &wsol_token_account)
            .unwrap(),
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
