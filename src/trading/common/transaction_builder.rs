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
    common::{nonce_cache::DurableNonceInfo, TradeTransactionVersion},
    trading::{
        core::transaction_pool::compile_message,
        MiddlewareManager,
    },
};

const V0_MAX_TRANSACTION_SIZE: usize = 1232;
const V1_MAX_TRANSACTION_SIZE: usize = solana_message::v1::MAX_TRANSACTION_SIZE;
const DEFAULT_INSTRUCTION_COMPUTE_UNIT_LIMIT: u32 = 200_000;
const MAX_COMPUTE_UNIT_LIMIT: u32 = 1_400_000;
const DEFAULT_LOADED_ACCOUNTS_DATA_SIZE_LIMIT: u32 = 64 * 1024 * 1024;
const MICRO_LAMPORTS_PER_LAMPORT: u128 = 1_000_000;

/// Convert SOL amount (f64) to lamports without string allocation (hot path).
#[inline(always)]
pub(crate) fn sol_f64_to_lamports(sol: f64) -> u64 {
    if sol <= 0.0 {
        return 0;
    }
    let lamports = sol * 1_000_000_000.0;
    (lamports.min(u64::MAX as f64)).round() as u64
}

/// Build signed transaction (worker hot path, no RPC).
/// Takes Arc/refs only; one Vec allocation (with_capacity), extend_from_slice for business_instructions, no extra clone of payer/middleware.
#[inline(always)]
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
    build_transaction_with_version(
        payer,
        unit_limit,
        unit_price,
        TradeTransactionVersion::V0,
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
    )
}

/// Build a signed transaction using the explicitly selected message version.
#[allow(clippy::too_many_arguments)]
pub fn build_transaction_with_version(
    payer: &Arc<Keypair>,
    unit_limit: u32,
    unit_price: u64,
    transaction_version: TradeTransactionVersion,
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
    if transaction_version == TradeTransactionVersion::V1
        && !address_lookup_table_accounts.is_empty()
    {
        return Err(anyhow!("V1 transactions do not support address lookup tables"));
    }

    let transaction = build_transaction_inner(
        payer,
        unit_limit,
        unit_price,
        transaction_version,
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

    let serialized_len = wincode::serialized_size(&transaction)? as usize;
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
    let max_transaction_size = match transaction_version {
        TradeTransactionVersion::V0 => V0_MAX_TRANSACTION_SIZE,
        TradeTransactionVersion::V1 => V1_MAX_TRANSACTION_SIZE,
    };
    if serialized_len <= max_transaction_size {
        return Ok(transaction);
    }

    match transaction_version {
        TradeTransactionVersion::V0 => Err(anyhow!(
            "transaction too large: {} > {}; SDK did not remove compute budget or relay tip because that changes transaction priority semantics. Use an address lookup table or pre-create token ATAs before submitting",
            serialized_len,
            max_transaction_size
        )),
        TradeTransactionVersion::V1 => Err(anyhow!(
            "transaction too large: {} > {}; SDK did not remove the relay tip or alter the V1 transaction config because that changes transaction priority semantics. Reduce instructions or pre-create token ATAs before submitting",
            serialized_len,
            max_transaction_size
        )),
    }
}

fn build_transaction_inner(
    payer: &Arc<Keypair>,
    unit_limit: u32,
    unit_price: u64,
    transaction_version: TradeTransactionVersion,
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

    if transaction_version == TradeTransactionVersion::V0 {
        super::compute_budget_manager::extend_compute_budget_instructions(
            &mut instructions,
            unit_price,
            unit_limit,
        );
    }

    instructions.extend_from_slice(business_instructions);

    let blockhash = get_transaction_blockhash(recent_blockhash, durable_nonce)?;

    build_versioned_transaction(
        payer,
        instructions,
        unit_limit,
        unit_price,
        transaction_version,
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
    unit_limit: u32,
    unit_price: u64,
    transaction_version: TradeTransactionVersion,
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

    let v1_config = if transaction_version == TradeTransactionVersion::V1 {
        let effective_unit_limit =
            effective_v1_compute_unit_limit(unit_limit, full_instructions.len());
        let mut config = solana_message::v1::TransactionConfig::empty()
            .with_compute_unit_limit(effective_unit_limit)
            .with_loaded_accounts_data_size_limit(DEFAULT_LOADED_ACCOUNTS_DATA_SIZE_LIMIT);
        if unit_price > 0 {
            config = config
                .with_priority_fee(v1_priority_fee_lamports(effective_unit_limit, unit_price));
        }
        config
    } else {
        solana_message::v1::TransactionConfig::empty()
    };

    // Compile directly from borrowed instructions; no intermediate deep clone
    // or shared compatibility-pool queue is needed on the sender hot path.
    let versioned_msg = compile_message(
        &payer.pubkey(),
        &full_instructions,
        address_lookup_table_accounts,
        blockhash,
        transaction_version,
        v1_config,
    )?;
    anyhow::ensure!(
        versioned_msg.header().num_required_signatures == 1
            && versioned_msg.static_account_keys().first() == Some(&payer.pubkey()),
        "Transaction requires unsupported additional signers; this builder signs only the payer"
    );

    let msg_bytes = versioned_msg.serialize();
    let signature =
        payer.as_ref().try_sign_message(&msg_bytes).map_err(|e| anyhow!("sign failed: {e}"))?;
    let tx = VersionedTransaction { signatures: vec![signature], message: versioned_msg };

    Ok(tx)
}

#[inline(always)]
fn effective_v1_compute_unit_limit(configured_limit: u32, instruction_count: usize) -> u32 {
    if configured_limit > 0 {
        return configured_limit;
    }
    u32::try_from(instruction_count)
        .unwrap_or(u32::MAX)
        .saturating_mul(DEFAULT_INSTRUCTION_COMPUTE_UNIT_LIMIT)
        .min(MAX_COMPUTE_UNIT_LIMIT)
}

#[inline(always)]
fn v1_priority_fee_lamports(unit_limit: u32, unit_price: u64) -> u64 {
    ((u128::from(unit_limit) * u128::from(unit_price))
        .saturating_add(MICRO_LAMPORTS_PER_LAMPORT - 1)
        / MICRO_LAMPORTS_PER_LAMPORT)
        .min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::{instruction::AccountMeta, message::VersionedMessage};

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

    fn build_test_transaction(
        version: TradeTransactionVersion,
        unit_limit: u32,
        unit_price: u64,
        business_instructions: &[Instruction],
        lookup_tables: &[AddressLookupTableAccount],
    ) -> Result<VersionedTransaction, anyhow::Error> {
        build_transaction_with_version(
            &Arc::new(Keypair::new()),
            unit_limit,
            unit_price,
            version,
            business_instructions,
            lookup_tables,
            Some(Hash::new_unique()),
            None,
            "test",
            true,
            false,
            &Pubkey::default(),
            0.0,
            None,
        )
    }

    #[test]
    fn v0_mode_without_lookup_tables_preserves_legacy_message() {
        let tx = build_test_transaction(TradeTransactionVersion::V0, 200_000, 1, &[], &[]).unwrap();
        let VersionedMessage::Legacy(message) = tx.message else {
            panic!("expected Legacy message");
        };
        assert!(message
            .account_keys
            .iter()
            .any(|key| *key == solana_compute_budget_interface::id()));
    }

    #[test]
    fn v0_mode_with_lookup_table_builds_v0_message() {
        let looked_up_address = Pubkey::new_unique();
        let instruction = Instruction {
            program_id: Pubkey::new_unique(),
            accounts: vec![AccountMeta::new_readonly(looked_up_address, false)],
            data: Vec::new(),
        };
        let lookup_table = AddressLookupTableAccount {
            key: Pubkey::new_unique(),
            addresses: vec![looked_up_address],
        };
        let tx = build_test_transaction(
            TradeTransactionVersion::V0,
            200_000,
            1,
            &[instruction],
            &[lookup_table],
        )
        .unwrap();
        assert!(matches!(tx.message, VersionedMessage::V0(_)));
    }

    #[test]
    fn v1_uses_inline_config_and_no_compute_budget_instruction() {
        let business_instruction =
            Instruction { program_id: Pubkey::new_unique(), accounts: Vec::new(), data: vec![1] };
        let tx = build_test_transaction(
            TradeTransactionVersion::V1,
            200_001,
            5,
            &[business_instruction],
            &[],
        )
        .unwrap();
        let VersionedMessage::V1(message) = tx.message else {
            panic!("expected V1 message");
        };

        assert_eq!(message.config.compute_unit_limit, Some(200_001));
        assert_eq!(message.config.priority_fee, Some(2));
        assert_eq!(
            message.config.loaded_accounts_data_size_limit,
            Some(DEFAULT_LOADED_ACCOUNTS_DATA_SIZE_LIMIT)
        );
        assert_eq!(message.config.heap_size, None);
        assert!(!message
            .account_keys
            .iter()
            .any(|key| *key == solana_compute_budget_interface::id()));
    }

    #[test]
    fn v1_zero_limit_matches_v0_default_compute_limit() {
        let instructions = vec![
            Instruction {
                program_id: Pubkey::new_unique(),
                accounts: Vec::new(),
                data: vec![]
            };
            8
        ];
        let tx =
            build_test_transaction(TradeTransactionVersion::V1, 0, 1, &instructions, &[]).unwrap();
        let VersionedMessage::V1(message) = tx.message else {
            panic!("expected V1 message");
        };
        assert_eq!(message.config.compute_unit_limit, Some(MAX_COMPUTE_UNIT_LIMIT));
        assert_eq!(message.config.priority_fee, Some(2));
    }

    #[test]
    fn v1_rejects_address_lookup_tables() {
        let lookup_table = AddressLookupTableAccount {
            key: Pubkey::new_unique(),
            addresses: vec![Pubkey::new_unique()],
        };
        let err =
            build_test_transaction(TradeTransactionVersion::V1, 200_000, 1, &[], &[lookup_table])
                .unwrap_err()
                .to_string();
        assert!(err.contains("V1 transactions do not support address lookup tables"), "{err}");
    }

    #[test]
    fn v1_accepts_payload_above_v0_limit_and_round_trips_with_wincode() {
        let instruction = oversized_instruction(1, 1_300);
        let v0_error = build_test_transaction(
            TradeTransactionVersion::V0,
            200_000,
            1,
            std::slice::from_ref(&instruction),
            &[],
        )
        .unwrap_err()
        .to_string();
        assert!(v0_error.contains("transaction too large"), "{v0_error}");

        let v1 =
            build_test_transaction(TradeTransactionVersion::V1, 200_000, 1, &[instruction], &[])
                .unwrap();
        let bytes = wincode::serialize(&v1).unwrap();
        assert!(bytes.len() > V0_MAX_TRANSACTION_SIZE);
        assert!(bytes.len() <= V1_MAX_TRANSACTION_SIZE);
        assert_eq!(wincode::serialized_size(&v1).unwrap() as usize, bytes.len());
        assert_eq!(bytes[0], solana_message::v1::V1_PREFIX);
        assert_eq!(&bytes[bytes.len() - 64..], v1.signatures[0].as_ref());
        v1.sanitize().unwrap();
        v1.verify_and_hash_message().unwrap();
        let decoded: VersionedTransaction = wincode::deserialize(&bytes).unwrap();
        assert_eq!(decoded, v1);
    }

    #[test]
    fn v1_rejects_payload_above_4096_bytes() {
        let instruction = oversized_instruction(1, 4_100);
        let err =
            build_test_transaction(TradeTransactionVersion::V1, 200_000, 1, &[instruction], &[])
                .unwrap_err()
                .to_string();
        assert!(err.contains("transaction too large"), "{err}");
        assert!(err.contains("4096"), "{err}");
    }

    #[test]
    fn v1_priority_fee_rounds_up_without_overflow() {
        assert_eq!(v1_priority_fee_lamports(200_001, 5), 2);
        assert_eq!(v1_priority_fee_lamports(u32::MAX, u64::MAX), u64::MAX);
    }
}
