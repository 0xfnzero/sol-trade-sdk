use std::sync::Arc;

use solana_sdk::{message::AddressLookupTableAccount, pubkey::Pubkey};

use crate::common::{
    address_lookup_cache::{get_address_lookup_table_account, AddressLookupTableCache},
    SolanaRpcClient,
};

/// Get address lookup table account list
/// If lookup_table_key is provided, get the corresponding account, otherwise return empty list
pub async fn get_address_lookup_table_accounts(
    rpc: Option<Arc<SolanaRpcClient>>,
    lookup_table_key: Option<Pubkey>,
) -> Vec<AddressLookupTableAccount> {
    match lookup_table_key {
        Some(key) => {
            let account = get_address_lookup_table_account(&key).await;
            if account.addresses.len() == 0 {
                if rpc.is_some() {
                    let _ = AddressLookupTableCache::get_instance()
                        .set_address_lookup_table(rpc.unwrap(), &key)
                        .await;
                    let new_account = get_address_lookup_table_account(&key).await;
                    if new_account.addresses.len() == 0 {
                        return Vec::new();
                    } else {
                        return vec![new_account];
                    }
                } else {
                    return Vec::new();
                }
            }
            return vec![account];
        }
        None => Vec::new(),
    }
}
