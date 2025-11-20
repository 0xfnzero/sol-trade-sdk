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

pub fn transfer(
    token_program_id: &Pubkey,
    source_pubkey: &Pubkey,
    destination_pubkey: &Pubkey,
    owner_pubkey: &Pubkey,
    amount: u64,
    signers: &[&Pubkey],
) -> Result<Instruction, ProgramError> {
    // Transfer
    let mut data = Vec::with_capacity(9);
    data.push(3); // Instruction discriminator for Transfer
    data.extend_from_slice(&amount.to_le_bytes());

    let mut accounts = Vec::with_capacity(3 + signers.len());
    accounts.push(AccountMeta::new(*source_pubkey, false));
    accounts.push(AccountMeta::new(*destination_pubkey, false));
    accounts.push(AccountMeta::new_readonly(*owner_pubkey, signers.is_empty()));

    for signer in signers.iter() {
        accounts.push(AccountMeta::new_readonly(**signer, true));
    }

    Ok(Instruction {
        program_id: *token_program_id,
        accounts,
        data,
    })
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
