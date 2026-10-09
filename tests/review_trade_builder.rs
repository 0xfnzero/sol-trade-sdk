use sol_trade_sdk::{
    common::{nonce_cache::DurableNonceInfo, TradeTransactionVersion},
    trading::{common::build_transaction_with_version, core::transaction_pool::acquire_builder},
};
use solana_hash::Hash;
use solana_message::{v1, AddressLookupTableAccount};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    message::{Message, VersionedMessage},
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

struct AllocationCounter;
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
// SAFETY: allocation ownership/layout are forwarded unchanged to System;
// the counter only observes calls and does not access allocated memory.
unsafe impl GlobalAlloc for AllocationCounter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        System.realloc(ptr, layout, size)
    }
}
#[global_allocator]
static ALLOCATOR: AllocationCounter = AllocationCounter;

// One test in its own executable: allocation comparisons have no other test thread.
#[test]
fn public_builder_rejects_invalid_inputs_and_compiles_without_instruction_clone() {
    sol_trade_sdk::common::sdk_log::set_sdk_log_enabled(false);
    let payer = Arc::new(Keypair::new());
    let hash = Hash::new_unique();
    let build = |version, ixs: &[Instruction], alts: &[AddressLookupTableAccount], nonce: Option<&DurableNonceInfo>| {
        build_transaction_with_version(
            &payer,
            200_000,
            0,
            version,
            ixs,
            alts,
            Some(hash),
            None,
            "regression",
            true,
            false,
            &Pubkey::default(),
            0.0,
            nonce,
        )
    };
    let extra = Keypair::new();
    let ix = Instruction {
        program_id: Pubkey::new_unique(),
        accounts: vec![AccountMeta::new_readonly(extra.pubkey(), true)],
        data: vec![1],
    };
    for version in [TradeTransactionVersion::V0, TradeTransactionVersion::V1] {
        assert!(build(version, std::slice::from_ref(&ix), &[], None)
            .unwrap_err()
            .to_string()
            .contains("additional signers"));
        let valid = build(version, &[], &[], None).unwrap();
        assert_eq!(valid.message.header().num_required_signatures, 1);
        assert_eq!(valid.message.static_account_keys().first(), Some(&payer.pubkey()));
        assert!(valid.sanitize().is_ok());
        assert!(valid.verify_and_hash_message().is_ok());
    }
    // The same check applies after ALT compilation, not only legacy messages.
    let alt = AddressLookupTableAccount {
        key: Pubkey::new_unique(),
        addresses: vec![Pubkey::new_unique()],
    };
    assert!(build(TradeTransactionVersion::V0, &[ix], std::slice::from_ref(&alt), None).is_err());
    let looked_up = Instruction {
        program_id: Pubkey::new_unique(),
        accounts: vec![AccountMeta::new(alt.addresses[0], false)],
        data: vec![2],
    };
    let tx = build(TradeTransactionVersion::V0, &[looked_up], &[alt], None).unwrap();
    assert!(matches!(&tx.message, VersionedMessage::V0(message)
        if !message.address_table_lookups.is_empty()));
    assert!(tx.sanitize().is_ok());
    assert!(tx.verify_and_hash_message().is_ok());
    for nonce in [
        DurableNonceInfo { nonce_account: None, current_nonce: Some(hash) },
        DurableNonceInfo { nonce_account: Some(Pubkey::new_unique()), current_nonce: None },
        DurableNonceInfo { nonce_account: None, current_nonce: None },
    ] {
        for version in [TradeTransactionVersion::V0, TradeTransactionVersion::V1] {
            assert!(build(version, &[], &[], Some(&nonce)).is_err());
        }
    }
    let nonce =
        DurableNonceInfo { nonce_account: Some(Pubkey::new_unique()), current_nonce: Some(hash) };
    let tx = build(TradeTransactionVersion::V0, &[], &[], Some(&nonce)).unwrap();
    assert!(tx.verify_and_hash_message().is_ok());
    assert_eq!(tx.message.recent_blockhash(), &hash);

    let ixs = [Instruction {
        program_id: Pubkey::new_unique(),
        accounts: (0..6).map(|_| AccountMeta::new(Pubkey::new_unique(), false)).collect(),
        data: vec![7; 16],
    }];
    let mut compatibility = acquire_builder();
    let key = payer.pubkey();
    ALLOCATIONS.store(0, Ordering::Relaxed);
    let direct = VersionedMessage::Legacy(Message::new_with_blockhash(&ixs, Some(&key), &hash));
    let direct_count = ALLOCATIONS.load(Ordering::Relaxed);
    ALLOCATIONS.store(0, Ordering::Relaxed);
    let compatible = compatibility
        .build_zero_alloc(
            &key,
            &ixs,
            &[],
            hash,
            TradeTransactionVersion::V0,
            v1::TransactionConfig::empty(),
        )
        .unwrap();
    let compatibility_count = ALLOCATIONS.load(Ordering::Relaxed);
    assert_eq!(direct, compatible);
    assert_eq!(
        compatibility_count, direct_count,
        "compatibility API must not deep-copy accounts/data"
    );
}
