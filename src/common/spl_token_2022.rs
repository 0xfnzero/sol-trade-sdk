use solana_program::pubkey;
use solana_sdk::{
    message::{AccountMeta, Instruction},
    program_error::ProgramError,
    pubkey::Pubkey,
};

pub const ID: Pubkey = pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

pub fn initialize_account3(
    token_program_id: &Pubkey,
    account_pubkey: &Pubkey,
    mint_pubkey: &Pubkey,
    owner_pubkey: &Pubkey,
) -> Result<Instruction, ProgramError> {
    // InitializeAccount3
    let mut data = Vec::with_capacity(33);
    data.push(18);
    data.extend_from_slice(owner_pubkey.as_ref());
    let accounts = vec![
        AccountMeta::new(*account_pubkey, false),
        AccountMeta::new_readonly(*mint_pubkey, false),
    ];
    Ok(Instruction { program_id: *token_program_id, accounts, data })
}
