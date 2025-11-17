use solana_hash::Hash;
use solana_sdk::{
    instruction::Instruction, message::AddressLookupTableAccount, native_token::sol_str_to_lamports, pubkey::Pubkey, signature::Keypair, signer::Signer, transaction::VersionedTransaction
};
use solana_system_interface::instruction::transfer;
use std::sync::Arc;

use super::{
    compute_budget_manager::compute_budget_instructions,
    nonce_manager::{add_nonce_instruction, get_transaction_blockhash},
};
use crate::{
    common::{nonce_cache::DurableNonceInfo, SolanaRpcClient},
    constants::swqos::NODE1_TIP_ACCOUNTS,
    trading::{MiddlewareManager, core::transaction_pool::{acquire_builder, release_builder}},
};

/// Build standard RPC transaction
pub async fn build_transaction(
    payer: Arc<Keypair>,
    rpc: Option<Arc<SolanaRpcClient>>,
    unit_limit: u32,
    unit_price: u64,
    business_instructions: Vec<Instruction>,
    address_lookup_table_account: Option<AddressLookupTableAccount>,
    recent_blockhash: Option<Hash>,
    data_size_limit: u32,
    middleware_manager: Option<Arc<MiddlewareManager>>,
    protocol_name: &str,
    is_buy: bool,
    with_tip: bool,
    tip_account: &Pubkey,
    tip_amount: f64,
    durable_nonce: Option<DurableNonceInfo>,
    // nonce_account: Option<Pubkey>,
    // current_nonce: Option<Hash>,
) -> Result<VersionedTransaction, anyhow::Error> {
    let mut instructions = Vec::with_capacity(business_instructions.len() + 5);

    // Add nonce instruction
    if let Err(e) =
        add_nonce_instruction(&mut instructions, payer.as_ref(), durable_nonce.clone())
    {
        return Err(e);
    }

    // Add tip transfer instruction
    if with_tip && tip_amount > 0.0 {
        // 🔧 Node1 最小小费金额限制：0.002 SOL（仅限 Node1）
        const MIN_TIP_AMOUNT: f64 = 0.002;

        // 检查是否是 Node1 的 tip_account
        let is_node1 = NODE1_TIP_ACCOUNTS.iter().any(|&account| account == *tip_account);

        let actual_tip_amount = if is_node1 && tip_amount < MIN_TIP_AMOUNT {
            // Node1 要求最小 0.002 SOL
            MIN_TIP_AMOUNT
        } else {
            // 其他 swqos 使用原始金额
            tip_amount
        };

        instructions.push(transfer(
            &payer.pubkey(),
            tip_account,
            sol_str_to_lamports(actual_tip_amount.to_string().as_str()).unwrap_or(0),
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
    let blockhash = get_transaction_blockhash(recent_blockhash, durable_nonce.clone());

    // Build transaction
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

/// Low-level function for building versioned transactions
async fn build_versioned_transaction(
    payer: Arc<Keypair>,
    instructions: Vec<Instruction>,
    address_lookup_table_account: Option<AddressLookupTableAccount>,
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

    // 使用预分配的交易构建器以降低延迟
    let mut builder = acquire_builder();

    let versioned_msg = builder.build_zero_alloc(
        &payer.pubkey(),
        &full_instructions,
        address_lookup_table_account,
        blockhash,
    );

    let msg_bytes = versioned_msg.serialize();
    let signature = payer.try_sign_message(&msg_bytes).expect("sign failed");
    let tx = VersionedTransaction { signatures: vec![signature], message: versioned_msg };

    // 归还构建器到池
    release_builder(builder);

    Ok(tx)
}
