use crate::common::SolanaRpcClient;
use solana_hash::Hash;
use solana_nonce::{state::State, versions::Versions};
use solana_sdk::pubkey::Pubkey;
use tracing::error;

/// DurableNonceInfo structure to store durable nonce-related information
#[derive(Clone)]
pub struct DurableNonceInfo {
    /// Nonce account address
    pub nonce_account: Option<Pubkey>,
    /// Current nonce value
    pub current_nonce: Option<Hash>,
}

/// Fetch nonce information using RPC
pub async fn fetch_nonce_info(
    rpc: &SolanaRpcClient,
    nonce_account: Pubkey,
) -> Option<DurableNonceInfo> {
    match rpc.get_account(&nonce_account).await {
        Ok(account) => {
            // Use the canonical state decoder. Legacy nonces cannot verify
            // current durable transactions, even when their state is initialized.
            if account.owner != solana_sdk::pubkey!("11111111111111111111111111111111")
                || account.executable
                || account.data.len() != State::size()
            {
                return None;
            }
            let versions: Versions = bincode::deserialize(&account.data).ok()?;
            if let Versions::Current(state) = versions {
                if let State::Initialized(data) = *state {
                    return Some(DurableNonceInfo {
                        nonce_account: Some(nonce_account),
                        current_nonce: Some(data.blockhash()),
                    });
                }
            }
        }
        Err(e) => {
            error!("Failed to get nonce account information: {:?}", e);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use solana_nonce::state::{Data, DurableNonce};
    use solana_rpc_client_api::request::RpcRequest;
    use solana_sdk::{signature::Keypair, signer::Signer};
    use std::sync::Arc;

    fn mock(owner: Pubkey, data: &[u8], executable: bool) -> SolanaRpcClient {
        let value = serde_json::json!({
            "context": {"slot": 100},
            "value": {"lamports": 1_000_000, "owner": owner.to_string(),
                "data": [STANDARD.encode(data), "base64"], "executable": executable,
                "rentEpoch": 0, "space": data.len()}
        });
        SolanaRpcClient::new_mock_with_mocks(
            "succeeds".into(),
            [(RpcRequest::GetAccountInfo, value)].into(),
        )
    }

    fn initialized(authority: Pubkey) -> Versions {
        Versions::new(State::Initialized(Data::new(
            authority,
            DurableNonce::from_blockhash(&Hash::new_unique()),
            8877,
        )))
    }

    #[tokio::test]
    async fn fetched_canonical_nonce_builds_a_valid_signed_transaction() {
        let payer = Arc::new(Keypair::new());
        let key = Pubkey::new_unique();
        let versions = initialized(payer.pubkey());
        let data = bincode::serialize(&versions).unwrap();
        assert_eq!(data.len(), State::size());
        let info = fetch_nonce_info(&mock(Pubkey::default(), &data, false), key).await.unwrap();
        assert_eq!(info.nonce_account, Some(key));
        assert!(versions.verify_recent_blockhash(&info.current_nonce.unwrap()).is_some());
        let tx = crate::trading::common::build_transaction_with_version(
            &payer,
            200_000,
            0,
            crate::common::TradeTransactionVersion::V0,
            &[],
            &[],
            None,
            None,
            "nonce-regression",
            true,
            false,
            &Pubkey::default(),
            0.0,
            Some(&info),
        )
        .unwrap();
        assert_eq!(tx.message.recent_blockhash(), &info.current_nonce.unwrap());
        tx.sanitize().unwrap();
        tx.verify_and_hash_message().unwrap();
    }

    #[tokio::test]
    async fn public_fetch_rejects_invalid_nonce_accounts() {
        let valid = bincode::serialize(&initialized(Pubkey::new_unique())).unwrap();
        let mut uninitialized = bincode::serialize(&Versions::new(State::Uninitialized)).unwrap();
        uninitialized.resize(State::size(), 0);
        let mut legacy = valid.clone();
        legacy[..4].copy_from_slice(&0u32.to_le_bytes());
        let mut unknown_version = valid.clone();
        unknown_version[..4].copy_from_slice(&2u32.to_le_bytes());
        let mut invalid_state = valid.clone();
        invalid_state[4..8].copy_from_slice(&2u32.to_le_bytes());
        let mut oversized = valid.clone();
        oversized.push(0);
        for (owner, bytes, executable) in [
            (Pubkey::default(), vec![0; 80], false),
            (Pubkey::default(), uninitialized, false),
            (Pubkey::default(), legacy, false),
            (Pubkey::default(), unknown_version, false),
            (Pubkey::default(), invalid_state, false),
            (Pubkey::default(), valid[..72].to_vec(), false),
            (Pubkey::default(), oversized, false),
            (Pubkey::new_unique(), valid.clone(), false),
            (Pubkey::default(), valid, true),
        ] {
            assert!(fetch_nonce_info(&mock(owner, &bytes, executable), Pubkey::new_unique())
                .await
                .is_none());
        }
    }
}
