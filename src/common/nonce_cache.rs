use crate::common::SolanaRpcClient;
use solana_hash::Hash;
use solana_nonce::state::State;
use solana_nonce::versions::Versions;
use solana_sdk::account_utils::StateMut;
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
        Ok(account) => match account.state() {
            Ok(Versions::Current(state)) => {
                if let State::Initialized(data) = *state {
                    let blockhash = data.durable_nonce.as_hash();
                    return Some(DurableNonceInfo {
                        nonce_account: Some(nonce_account),
                        current_nonce: Some(*blockhash),
                    });
                }
            }
            _ => (),
        },
        Err(e) => {
            error!("Failed to get nonce account information: {:?}", e);
        }
    }
    None
}
