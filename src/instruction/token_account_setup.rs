use solana_sdk::{instruction::Instruction, pubkey::Pubkey};

#[inline]
pub(crate) fn push_create_user_token_account(
    instructions: &mut Vec<Instruction>,
    payer: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
    use_seed: bool,
) {
    instructions.extend(
        crate::common::fast_fn::create_associated_token_account_idempotent_fast_use_seed(
            payer,
            payer,
            mint,
            token_program,
            use_seed,
        ),
    );
}

#[inline]
pub(crate) fn push_create_or_wrap_user_token_account(
    instructions: &mut Vec<Instruction>,
    payer: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
    amount: u64,
    use_seed: bool,
) {
    if *mint == crate::constants::WSOL_TOKEN_ACCOUNT {
        instructions.extend(crate::trading::common::handle_wsol(payer, amount));
    } else {
        push_create_user_token_account(instructions, payer, mint, token_program, use_seed);
    }
}

#[inline]
pub(crate) fn push_close_wsol_if_needed(
    instructions: &mut Vec<Instruction>,
    payer: &Pubkey,
    mint: &Pubkey,
) {
    if *mint == crate::constants::WSOL_TOKEN_ACCOUNT {
        instructions.extend(crate::trading::common::close_wsol(payer));
    }
}
