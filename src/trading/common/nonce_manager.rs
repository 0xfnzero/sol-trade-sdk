use solana_hash::Hash;
use solana_sdk::{instruction::Instruction, signature::Keypair, signer::Signer};
use solana_system_interface::instruction::advance_nonce_account;

use crate::common::nonce_cache::DurableNonceInfo;

/// Add nonce advance instruction to the instruction set
///
/// Nonce functionality is only used when nonce_pubkey is provided
/// Returns error if nonce is locked, already used, or not ready
/// On success, locks and marks nonce as used
pub fn add_nonce_instruction(
    instructions: &mut Vec<Instruction>,
    payer: &Keypair,
    durable_nonce: Option<&DurableNonceInfo>,
) -> Result<(), anyhow::Error> {
    if let Some(durable_nonce) = durable_nonce {
        let nonce_advance_ix =
            advance_nonce_account(&durable_nonce.nonce_account.unwrap(), &payer.pubkey());
        instructions.push(nonce_advance_ix);
    }

    Ok(())
}

/// Get blockhash for transaction.
/// If nonce account is used, returns blockhash from nonce; otherwise returns the provided recent_blockhash.
/// Returns error when neither durable_nonce nor recent_blockhash is set (caller must provide one for low latency).
pub fn get_transaction_blockhash(
    recent_blockhash: Option<Hash>,
    durable_nonce: Option<&DurableNonceInfo>,
) -> Result<Hash, anyhow::Error> {
    if let Some(durable_nonce) = durable_nonce {
        durable_nonce
            .current_nonce
            .ok_or_else(|| anyhow::anyhow!("durable_nonce.current_nonce is None"))
    } else if let Some(hash) = recent_blockhash {
        Ok(hash)
    } else {
        Err(anyhow::anyhow!(
            "Must provide either recent_blockhash or durable_nonce for transaction"
        ))
    }
}
