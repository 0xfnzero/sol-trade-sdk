use solana_hash::Hash;
use solana_sdk::{
    instruction::Instruction, message::AddressLookupTableAccount, pubkey::Pubkey,
    signature::Keypair, signer::Signer, transaction::VersionedTransaction,
};
use solana_system_interface::instruction::transfer;
use std::sync::Arc;

use super::nonce_manager::{add_nonce_instruction, get_transaction_blockhash};
use crate::{
    common::{nonce_cache::DurableNonceInfo, SolanaRpcClient},
    trading::{
        core::transaction_pool::{acquire_builder, release_builder},
        MiddlewareManager,
    },
};

/// Convert SOL amount (f64) to lamports without string allocation (hot path).
#[inline(always)]
fn sol_f64_to_lamports(sol: f64) -> u64 {
    if sol <= 0.0 {
        return 0;
    }
    let lamports = sol * 1_000_000_000.0;
    (lamports.min(u64::MAX as f64)).round() as u64
}

/// Build standard RPC transaction.
/// Takes Arc/context by reference to avoid clone in worker hot path (Arc::clone is cheap but ref is zero-cost).
pub async fn build_transaction(
    payer: &Arc<Keypair>,
    _rpc: Option<&Arc<SolanaRpcClient>>,
    unit_limit: u32,
    unit_price: u64,
    business_instructions: &[Instruction],
    address_lookup_table_account: Option<&AddressLookupTableAccount>,
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
        instructions.push(transfer(&payer.pubkey(), tip_account, tip_lamports));
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
        address_lookup_table_account,
        blockhash,
        middleware_manager,
        protocol_name,
        is_buy,
    )
    .await
}

async fn build_versioned_transaction(
    payer: &Arc<Keypair>,
    instructions: Vec<Instruction>,
    address_lookup_table_account: Option<&AddressLookupTableAccount>,
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

    let versioned_msg = builder.build_zero_alloc(
        &payer.pubkey(),
        &full_instructions,
        address_lookup_table_account,
        blockhash,
    );

    let msg_bytes = versioned_msg.serialize();
    let signature = payer.as_ref().try_sign_message(&msg_bytes).expect("sign failed");
    let tx = VersionedTransaction { signatures: vec![signature], message: versioned_msg };

    // 归还构建器到池
    release_builder(builder);

    Ok(tx)
}
