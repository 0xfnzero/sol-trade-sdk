use sol_trade_sdk::{
    common::{nonce_cache::DurableNonceInfo, TradeTransactionVersion},
    trading::common::build_transaction_with_version,
};
use solana_hash::Hash;
use solana_sdk::{pubkey::Pubkey, signature::Keypair, signer::Signer};
use std::sync::Arc;

#[test]
fn cached_nonce_and_alt_resolve_signed_instruction_accounts_without_rpc() {
    use solana_message::AddressLookupTableAccount;
    use solana_sdk::{message::VersionedMessage, transaction::VersionedTransaction};
    let payer = Arc::new(Keypair::new());
    let nonce_account = Pubkey::new_unique();
    let recipient = Pubkey::new_unique();
    let nonce_hash = Hash::new_unique();
    let nonce = DurableNonceInfo { nonce_account: Some(nonce_account), current_nonce: Some(nonce_hash) };
    let table = AddressLookupTableAccount { key: Pubkey::new_unique(), addresses: vec![recipient, nonce_account] };
    let transfer = solana_system_interface::instruction::transfer(&payer.pubkey(), &recipient, 7);
    let tx = build_transaction_with_version(&payer, 200_000, 1, TradeTransactionVersion::V0,
        &[transfer], std::slice::from_ref(&table), Some(Hash::new_unique()), None,
        "nonce-alt-boundary", true, true, &Pubkey::new_unique(), 0.0001, Some(&nonce)).unwrap();
    let parsed: VersionedTransaction = wincode::deserialize_exact(&wincode::serialize(&tx).unwrap()).unwrap();
    parsed.sanitize().unwrap(); parsed.verify_and_hash_message().unwrap();
    assert_eq!(parsed.message.recent_blockhash(), &nonce_hash);
    let VersionedMessage::V0(message) = &parsed.message else { panic!("expected ALT-backed V0"); };
    assert_eq!(message.address_table_lookups.len(), 1);
    assert_eq!(message.address_table_lookups[0].account_key, table.key);
    let mut resolved = message.account_keys.clone();
    for lookup in &message.address_table_lookups {
        resolved.extend(lookup.writable_indexes.iter().map(|i| table.addresses[*i as usize]));
    }
    for lookup in &message.address_table_lookups {
        resolved.extend(lookup.readonly_indexes.iter().map(|i| table.addresses[*i as usize]));
    }
    let advance = &message.instructions[0];
    assert_eq!(advance.data, [4, 0, 0, 0]);
    assert_eq!(resolved[advance.program_id_index as usize], Pubkey::default());
    assert_eq!(resolved[advance.accounts[0] as usize], nonce_account);
    assert_eq!(resolved[advance.accounts[2] as usize], payer.pubkey());
    let transfer = message.instructions.last().unwrap();
    assert_eq!(resolved[transfer.accounts[1] as usize], recipient);
    assert_eq!(u64::from_le_bytes(transfer.data[4..12].try_into().unwrap()), 7);
}

#[test]
fn durable_nonce_is_first_and_signed_for_both_versions_without_rpc() {
    let payer = Arc::new(Keypair::new());
    let nonce = DurableNonceInfo {
        nonce_account: Some(Pubkey::new_unique()),
        current_nonce: Some(Hash::new_unique()),
    };
    for version in [TradeTransactionVersion::V0, TradeTransactionVersion::V1] {
        let build = |nonce: Option<&DurableNonceInfo>| {
            build_transaction_with_version(
                &payer,
                200_000,
                1,
                version,
                &[],
                &[],
                None,
                None,
                "nonce-boundary",
                true,
                false,
                &Pubkey::default(),
                0.0,
                nonce,
            )
        };
        let tx = build(Some(&nonce)).unwrap();
        assert_eq!(tx.message.recent_blockhash(), &nonce.current_nonce.unwrap());
        let first = &tx.message.instructions()[0];
        assert_eq!(
            tx.message.static_account_keys()[usize::from(first.program_id_index)],
            Pubkey::default()
        );
        assert_eq!(first.data, [4, 0, 0, 0]);
        tx.sanitize().unwrap();
        assert!(tx.signatures[0].verify(payer.pubkey().as_ref(), &tx.message.serialize()));
        let mut damaged = tx.message.serialize();
        *damaged.last_mut().unwrap() ^= 1;
        assert!(!tx.signatures[0].verify(payer.pubkey().as_ref(), &damaged));
        assert!(build(None).is_err());
        assert!(build(Some(&DurableNonceInfo {
            nonce_account: None,
            current_nonce: nonce.current_nonce
        }))
        .is_err());
        assert!(build(Some(&DurableNonceInfo {
            nonce_account: nonce.nonce_account,
            current_nonce: None
        }))
        .is_err());
    }
}
