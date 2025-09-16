use solana_hash::Hash;
use solana_sdk::{
    instruction::Instruction,
    message::{v0, VersionedMessage},
    native_token::sol_str_to_lamports,
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    transaction::VersionedTransaction,
};
use solana_system_interface::instruction::transfer;
use std::sync::Arc;

use super::{
    address_lookup_manager::get_address_lookup_table_accounts,
    compute_budget_manager::compute_budget_instructions,
    nonce_manager::{add_nonce_instruction, get_transaction_blockhash},
};
use crate::trading::MiddlewareManager;

/// Build standard RPC transaction
pub async fn build_transaction(
    payer: Arc<Keypair>,
    unit_limit: u32,
    unit_price: u64,
    business_instructions: Vec<Instruction>,
    lookup_table_key: Option<Pubkey>,
    recent_blockhash: Hash,
    data_size_limit: u32,
    middleware_manager: Option<Arc<MiddlewareManager>>,
    protocol_name: &str,
    is_buy: bool,
    with_tip: bool,
    tip_account: &Pubkey,
    tip_amount: f64,
) -> Result<VersionedTransaction, anyhow::Error> {
    let mut instructions = Vec::with_capacity(business_instructions.len() + 5);

    // Add nonce instruction
    if let Err(e) = add_nonce_instruction(&mut instructions, payer.as_ref()) {
        return Err(e);
    }

    // Add tip transfer instruction
    if with_tip && tip_amount > 0.0 {
        instructions.push(transfer(
            &payer.pubkey(),
            tip_account,
            sol_str_to_lamports(tip_amount.to_string().as_str()).unwrap_or(0),
        ));
    }

    // Add compute budget instructions
    instructions.extend(compute_budget_instructions(
        unit_price,
        unit_limit,
        data_size_limit,
        is_buy,
    ));

    // Add business instructions
    instructions.extend(business_instructions);

    // Get blockhash for transaction
    let blockhash =
        if is_buy { get_transaction_blockhash(recent_blockhash) } else { recent_blockhash };

    // Get address lookup table accounts
    let address_lookup_table_accounts = get_address_lookup_table_accounts(lookup_table_key).await;

    // Build transaction
    build_versioned_transaction(
        payer,
        instructions,
        address_lookup_table_accounts,
        blockhash,
        middleware_manager,
        protocol_name,
        is_buy,
    )
    .await
}

/// Low-level function for building versioned transactions
async fn build_versioned_transaction(
    payer: Arc<Keypair>,
    instructions: Vec<Instruction>,
    address_lookup_table_accounts: Vec<solana_sdk::message::AddressLookupTableAccount>,
    blockhash: Hash,
    middleware_manager: Option<Arc<MiddlewareManager>>,
    protocol_name: &str,
    is_buy: bool,
) -> Result<VersionedTransaction, anyhow::Error> {
    let full_instructions = match middleware_manager {
        Some(middleware_manager) => middleware_manager
            .apply_middlewares_process_full_instructions(
                instructions,
                protocol_name.to_string(),
                is_buy,
            )?,
        None => instructions,
    };
    let v0_message: v0::Message = v0::Message::try_compile(
        &payer.pubkey(),
        &full_instructions,
        &address_lookup_table_accounts,
        blockhash,
    )?;
    let versioned_msg = VersionedMessage::V0(v0_message);
    let msg_bytes = versioned_msg.serialize();
    let signature = payer.try_sign_message(&msg_bytes).expect("sign failed");
    Ok(VersionedTransaction { signatures: vec![signature], message: versioned_msg })
}
