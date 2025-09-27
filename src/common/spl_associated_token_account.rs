use solana_sdk::{
    message::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

use crate::constants::{ASSOCIATED_TOKEN_PROGRAM_ID, SYSTEM_PROGRAM};

pub fn get_associated_token_address_with_program_id(
    wallet_address: &Pubkey,
    token_mint_address: &Pubkey,
    token_program_id: &Pubkey,
) -> Pubkey {
    Pubkey::find_program_address(
        &[&wallet_address.to_bytes(), &token_program_id.to_bytes(), &token_mint_address.to_bytes()],
        &crate::constants::ASSOCIATED_TOKEN_PROGRAM_ID,
    )
    .0
}

pub fn get_associated_token_address(
    wallet_address: &Pubkey,
    token_mint_address: &Pubkey,
) -> Pubkey {
    get_associated_token_address_with_program_id(
        wallet_address,
        token_mint_address,
        &crate::constants::TOKEN_PROGRAM,
    )
}

pub fn create_associated_token_account_idempotent(
    funding_address: &Pubkey,
    wallet_address: &Pubkey,
    token_mint_address: &Pubkey,
    token_program_id: &Pubkey,
) -> Instruction {
    let instruction = 1;
    let associated_account_address = get_associated_token_address_with_program_id(
        wallet_address,
        token_mint_address,
        token_program_id,
    );
    Instruction {
        program_id: ASSOCIATED_TOKEN_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(*funding_address, true),
            AccountMeta::new(associated_account_address, false),
            AccountMeta::new_readonly(*wallet_address, false),
            AccountMeta::new_readonly(*token_mint_address, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
            AccountMeta::new_readonly(*token_program_id, false),
        ],
        data: vec![instruction],
    }
}
