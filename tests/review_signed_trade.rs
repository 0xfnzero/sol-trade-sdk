use sol_trade_sdk::{
    common::TradeTransactionVersion, trading::common::build_transaction_with_version,
};
use solana_hash::Hash;
use solana_message::AddressLookupTableAccount;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    transaction::VersionedTransaction,
};
use std::sync::Arc;

#[test]
fn generated_key_signatures_parse_and_reject_message_and_signature_tampering() {
    let payer = Arc::new(Keypair::new());
    let build = |version, instructions: &[Instruction], tables: &[AddressLookupTableAccount]| {
        build_transaction_with_version(
            &payer,
            200_000,
            0,
            version,
            instructions,
            tables,
            Some(Hash::new_unique()),
            None,
            "signed-regression",
            true,
            false,
            &Pubkey::default(),
            0.0,
            None,
        )
    };
    let looked_up = Pubkey::new_unique();
    let ix = Instruction {
        program_id: Pubkey::new_unique(),
        accounts: vec![AccountMeta::new(payer.pubkey(), true), AccountMeta::new(looked_up, false)],
        data: vec![1, 2, 3],
    };
    let alt = AddressLookupTableAccount { key: Pubkey::new_unique(), addresses: vec![looked_up] };
    for (version, tables) in [
        (TradeTransactionVersion::V0, vec![]),
        (TradeTransactionVersion::V0, vec![alt]),
        (TradeTransactionVersion::V1, vec![]),
    ] {
        let tx = build(version, std::slice::from_ref(&ix), &tables).unwrap();
        if !tables.is_empty() {
            assert!(
                matches!(&tx.message, solana_sdk::message::VersionedMessage::V0(m) if !m.address_table_lookups.is_empty())
            );
        }
        let wire = wincode::serialize(&tx).unwrap();
        let parsed: VersionedTransaction = wincode::deserialize(&wire).unwrap();
        assert!(parsed.sanitize().is_ok());
        assert!(parsed.verify_and_hash_message().is_ok());
        assert_eq!(parsed.message.static_account_keys()[0], payer.pubkey());
        let message = parsed.message.serialize();
        assert!(parsed.signatures[0].verify(payer.pubkey().as_ref(), &message));
        let mut tampered_message = message.clone();
        *tampered_message.last_mut().unwrap() ^= 1;
        assert!(!parsed.signatures[0].verify(payer.pubkey().as_ref(), &tampered_message));
        let mut bad = parsed.clone();
        let mut signature = <[u8; 64]>::from(bad.signatures[0]);
        signature[0] ^= 1;
        bad.signatures[0] = signature.into();
        assert!(bad.verify_and_hash_message().is_err());
        let extra = Instruction {
            accounts: vec![AccountMeta::new_readonly(Keypair::new().pubkey(), true)],
            ..ix.clone()
        };
        assert!(build(version, &[extra], &tables)
            .unwrap_err()
            .to_string()
            .contains("additional signers"));
    }
}
