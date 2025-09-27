use solana_program::pubkey;
use solana_sdk::{
    message::{AccountMeta, Instruction},
    program_error::ProgramError,
    pubkey::Pubkey,
};

pub const ID: Pubkey = pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

pub fn close_account(
    token_program_id: &Pubkey,
    account_pubkey: &Pubkey,
    destination_pubkey: &Pubkey,
    owner_pubkey: &Pubkey,
    signer_pubkeys: &[&Pubkey],
) -> Result<Instruction, solana_sdk::program_error::ProgramError> {
    // CloseAccount
    let mut data = Vec::with_capacity(1);
    data.push(9);
    let mut accounts = Vec::with_capacity(3 + signer_pubkeys.len());
    accounts.push(solana_sdk::message::AccountMeta::new(*account_pubkey, false));
    accounts.push(solana_sdk::message::AccountMeta::new(*destination_pubkey, false));
    accounts.push(solana_sdk::message::AccountMeta::new_readonly(
        *owner_pubkey,
        signer_pubkeys.is_empty(),
    ));
    for signer_pubkey in signer_pubkeys.iter() {
        accounts.push(solana_sdk::message::AccountMeta::new_readonly(**signer_pubkey, true));
    }
    Ok(Instruction { program_id: *token_program_id, accounts, data })
}

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
