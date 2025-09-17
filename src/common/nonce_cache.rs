use parking_lot::Mutex;
use solana_hash::Hash;
use solana_sdk::account_utils::StateMut;
use solana_sdk::nonce::state::Versions;
use solana_sdk::nonce::State;
use solana_sdk::pubkey::Pubkey;
use solana_streamer_sdk::common::SolanaRpcClient;
use std::str::FromStr;
use std::sync::{Arc, OnceLock};
use tracing::error;

/// NonceInfo structure to store nonce-related information
pub struct NonceInfo {
    /// Nonce account address
    pub nonce_account: Option<Pubkey>,
    /// Current nonce value
    pub current_nonce: Hash,
    /// Next available time (Unix timestamp in seconds)
    pub next_buy_time: i64,
    /// Whether it has been used
    pub used: bool,
}

/// NonceInfoStore singleton for storing and managing NonceInfo
pub struct NonceCache {
    /// Internally stored NonceInfo data
    nonce_info: Mutex<NonceInfo>,
}

// Use static OnceLock to ensure thread safety of singleton pattern
static NONCE_CACHE: OnceLock<Arc<NonceCache>> = OnceLock::new();

impl NonceCache {
    /// Get NonceInfoStore singleton instance
    pub fn get_instance() -> Arc<NonceCache> {
        NONCE_CACHE
            .get_or_init(|| {
                Arc::new(NonceCache {
                    nonce_info: Mutex::new(NonceInfo {
                        nonce_account: None,
                        current_nonce: Hash::default(),
                        next_buy_time: 0,
                        used: false,
                    }),
                })
            })
            .clone()
    }

    /// Initialize nonce information
    pub fn init(&self, nonce_account_str: Option<String>) {
        let nonce_account = nonce_account_str.and_then(|s| Pubkey::from_str(&s).ok());
        self.update_nonce_info_partial(nonce_account, None, None, Some(false));
    }

    /// Get a copy of NonceInfo
    pub fn get_nonce_info(&self) -> NonceInfo {
        let nonce_info = self.nonce_info.lock();
        NonceInfo {
            nonce_account: nonce_info.nonce_account,
            current_nonce: nonce_info.current_nonce,
            next_buy_time: nonce_info.next_buy_time,
            used: nonce_info.used,
        }
    }

    /// Partially update NonceInfo, only update the passed fields
    pub fn update_nonce_info_partial(
        &self,
        nonce_account: Option<Pubkey>,
        current_nonce: Option<Hash>,
        next_buy_time: Option<i64>,
        used: Option<bool>,
    ) {
        let mut current = self.nonce_info.lock();

        // Only update the passed fields
        if let Some(account) = nonce_account {
            current.nonce_account = Some(account);
        }

        if let Some(nonce) = current_nonce {
            current.current_nonce = nonce;
        }

        if let Some(time) = next_buy_time {
            current.next_buy_time = time;
        }

        if let Some(u) = used {
            current.used = u;
        }
    }

    /// Mark nonce as used
    pub fn mark_used(&self) {
        self.update_nonce_info_partial(None, None, None, Some(true));
    }

    /// Fetch nonce information using RPC
    pub async fn fetch_nonce_info_use_rpc(
        &self,
        rpc: &SolanaRpcClient,
    ) -> Result<(), anyhow::Error> {
        match rpc.get_account(&self.get_nonce_info().nonce_account.unwrap()).await {
            Ok(account) => match account.state() {
                Ok(Versions::Current(state)) => {
                    if let State::Initialized(data) = *state {
                        let blockhash = data.durable_nonce.as_hash();
                        let old_nonce_info = self.get_nonce_info();
                        if old_nonce_info.current_nonce != *blockhash {
                            self.update_nonce_info_partial(
                                None,
                                Some(*blockhash),
                                None,
                                Some(false),
                            );
                        }
                    }
                }
                _ => (),
            },
            Err(e) => {
                error!("Failed to get nonce account information: {:?}", e);
            }
        }
        Ok(())
    }
}
