use anyhow::anyhow;
use solana_hash::Hash;
use solana_message::AddressLookupTableAccount;
use solana_sdk::{
    instruction::Instruction, pubkey::Pubkey, signature::Keypair, signer::Signer,
    transaction::VersionedTransaction,
};
use solana_system_interface::instruction as system_instruction;
use std::sync::Arc;

use super::nonce_manager::{add_nonce_instruction, get_transaction_blockhash};
use crate::{
    common::nonce_cache::DurableNonceInfo,
    trading::{
        core::transaction_pool::{acquire_builder, release_builder},
        MiddlewareManager,
    },
};

const PACKET_DATA_SIZE: usize = 1232;

/// Convert SOL amount (f64) to lamports without string allocation (hot path).
#[inline(always)]
fn sol_f64_to_lamports(sol: f64) -> u64 {
    if sol <= 0.0 {
        return 0;
    }
    let lamports = sol * 1_000_000_000.0;
    (lamports.min(u64::MAX as f64)).round() as u64
}

/// Build signed transaction (worker hot path, no RPC).
/// Takes Arc/refs only; one Vec allocation (with_capacity), extend_from_slice for business_instructions, no extra clone of payer/middleware.
pub fn build_transaction(
    payer: &Arc<Keypair>,
    unit_limit: u32,
    unit_price: u64,
    business_instructions: &[Instruction],
    address_lookup_table_accounts: &[AddressLookupTableAccount],
    recent_blockhash: Option<Hash>,
    middleware_manager: Option<&Arc<MiddlewareManager>>,
    protocol_name: &str,
    is_buy: bool,
    with_tip: bool,
    tip_account: &Pubkey,
    tip_amount: f64,
    durable_nonce: Option<&DurableNonceInfo>,
) -> Result<VersionedTransaction, anyhow::Error> {
    let transaction = build_transaction_inner(
        payer,
        unit_limit,
        unit_price,
        business_instructions,
        address_lookup_table_accounts,
        recent_blockhash,
        middleware_manager,
        protocol_name,
        is_buy,
        with_tip,
        tip_account,
        tip_amount,
        durable_nonce,
    )?;

    let serialized_len = bincode::serialized_size(&transaction)? as usize;
    if crate::common::sdk_log::sdk_log_enabled() {
        println!(
            " [SDK][tx-size     ] {} {} serialized={} bytes, business_ix={}, nonce={}, tip={}, cu_limit={}, cu_price={}, alt={}",
            protocol_name,
            if is_buy { "buy" } else { "sell" },
            serialized_len,
            business_instructions.len(),
            durable_nonce.is_some(),
            with_tip && tip_amount > 0.0,
            unit_limit,
            unit_price,
            address_lookup_table_accounts.len()
        );
    }
    if serialized_len <= PACKET_DATA_SIZE {
        return Ok(transaction);
    }

    Err(anyhow!(
        "transaction too large: {} > {}; SDK did not remove compute budget or relay tip because that changes transaction priority semantics. Use an address lookup table or pre-create token ATAs before submitting",
        serialized_len,
        PACKET_DATA_SIZE
    ))
}

fn build_transaction_inner(
    payer: &Arc<Keypair>,
    unit_limit: u32,
    unit_price: u64,
    business_instructions: &[Instruction],
    address_lookup_table_accounts: &[AddressLookupTableAccount],
    recent_blockhash: Option<Hash>,
    middleware_manager: Option<&Arc<MiddlewareManager>>,
    protocol_name: &str,
    is_buy: bool,
    with_tip: bool,
    tip_account: &Pubkey,
    tip_amount: f64,
    durable_nonce: Option<&DurableNonceInfo>,
) -> Result<VersionedTransaction, anyhow::Error> {
    let mut instructions = Vec::with_capacity(business_instructions.len() + 5);

    if let Err(e) = add_nonce_instruction(&mut instructions, payer.as_ref(), durable_nonce) {
        return Err(e);
    }

    if with_tip && tip_amount > 0.0 {
        let tip_lamports = sol_f64_to_lamports(tip_amount);
        instructions.push(system_instruction::transfer(&payer.pubkey(), tip_account, tip_lamports));
    }

    super::compute_budget_manager::extend_compute_budget_instructions(
        &mut instructions,
        unit_price,
        unit_limit,
    );

    instructions.extend_from_slice(business_instructions);

    let blockhash = get_transaction_blockhash(recent_blockhash, durable_nonce)?;

    build_versioned_transaction(
        payer,
        instructions,
        address_lookup_table_accounts,
        blockhash,
        middleware_manager,
        protocol_name,
        is_buy,
    )
}

fn build_versioned_transaction(
    payer: &Arc<Keypair>,
    instructions: Vec<Instruction>,
    address_lookup_table_accounts: &[AddressLookupTableAccount],
    blockhash: Hash,
    middleware_manager: Option<&Arc<MiddlewareManager>>,
    protocol_name: &str,
    is_buy: bool,
) -> Result<VersionedTransaction, anyhow::Error> {
    let full_instructions = match middleware_manager {
        Some(middleware_manager) => middleware_manager
            .apply_middlewares_process_full_instructions(instructions, protocol_name, is_buy)?,
        None => instructions,
    };

    // 使用预分配的交易构建器以降低延迟
    let mut builder = acquire_builder();

    let build_result = builder.build_zero_alloc(
        &payer.pubkey(),
        &full_instructions,
        address_lookup_table_accounts,
        blockhash,
    );
    release_builder(builder);
    let versioned_msg = build_result?;

    let msg_bytes = versioned_msg.serialize();
    let signature =
        payer.as_ref().try_sign_message(&msg_bytes).map_err(|e| anyhow!("sign failed: {e}"))?;
    let tx = VersionedTransaction { signatures: vec![signature], message: versioned_msg };

    Ok(tx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::instruction::AccountMeta;

    fn oversized_instruction(account_count: usize, data_len: usize) -> Instruction {
        let accounts =
            (0..account_count).map(|_| AccountMeta::new(Pubkey::new_unique(), false)).collect();
        Instruction { program_id: Pubkey::new_unique(), accounts, data: vec![7; data_len] }
    }

    #[test]
    fn oversized_transaction_returns_error_without_dropping_priority_semantics() {
        let payer = Arc::new(Keypair::new());
        let business_instructions = vec![oversized_instruction(36, 700)];
        let err = build_transaction(
            &payer,
            80_000,
            100_000,
            &business_instructions,
            &[],
            Some(Hash::new_unique()),
            None,
            "test",
            true,
            true,
            &Pubkey::new_unique(),
            0.001,
            None,
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains("transaction too large"), "{err}");
        assert!(err.contains("did not remove compute budget or relay tip"), "{err}");
    }
}
