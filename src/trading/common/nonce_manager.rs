use solana_hash::Hash;
use solana_sdk::{instruction::Instruction, pubkey::Pubkey, signature::Keypair, signer::Signer};
use solana_system_interface::instruction::advance_nonce_account;

/// Add nonce advance instruction to the instruction set
///
/// Nonce functionality is only used when nonce_pubkey is provided
/// Returns error if nonce is locked, already used, or not ready
/// On success, locks and marks nonce as used
pub fn add_nonce_instruction(
    instructions: &mut Vec<Instruction>,
    payer: &Keypair,
    nonce_account: Option<Pubkey>,
    current_nonce: Option<Hash>,
) -> Result<(), anyhow::Error> {
    if nonce_account.is_some() && current_nonce.is_some() {
        let nonce_advance_ix = advance_nonce_account(&nonce_account.unwrap(), &payer.pubkey());
        instructions.push(nonce_advance_ix);
    }
    Ok(())
}

/// Get blockhash for transaction
/// If nonce account is used, return blockhash from nonce, otherwise return the provided recent_blockhash
pub fn get_transaction_blockhash(
    recent_blockhash: Hash,
    nonce_account: Option<Pubkey>,
    current_nonce: Option<Hash>,
) -> Hash {
    if nonce_account.is_some() && current_nonce.is_some() {
        current_nonce.unwrap()
    } else {
        recent_blockhash
    }
}
