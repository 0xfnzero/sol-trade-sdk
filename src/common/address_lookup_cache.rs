use dashmap::DashMap;
use solana_sdk::{
    address_lookup_table::state::AddressLookupTable, message::AddressLookupTableAccount,
    pubkey::Pubkey,
};
use std::{
    error::Error,
    sync::{Arc, OnceLock},
};

use crate::common::SolanaRpcClient;

/// AddressLookupTableInfo struct, stores address lookup table related information
#[derive(Clone)]
pub struct AddressLookupTableInfo {
    /// Address lookup table account address
    pub lookup_table_address: Option<Pubkey>,
    /// Address lookup table content
    pub address_lookup_table: Option<AddressLookupTableAccount>,
}

/// AddressLookupTableCache singleton for storing and managing address lookup tables
pub struct AddressLookupTableCache {
    /// Lock-free hash map supporting high concurrent access
    tables: DashMap<Pubkey, AddressLookupTableInfo>,
}

// Use static OnceLock to ensure thread safety of singleton pattern
static ADDRESS_LOOKUP_TABLE_CACHE: OnceLock<Arc<AddressLookupTableCache>> = OnceLock::new();

impl AddressLookupTableCache {
    /// Get AddressLookupTableCache singleton instance
    pub fn get_instance() -> Arc<AddressLookupTableCache> {
        ADDRESS_LOOKUP_TABLE_CACHE
            .get_or_init(|| Arc::new(AddressLookupTableCache { tables: DashMap::new() }))
            .clone()
    }

    /// Get lookup table information
    pub async fn set_address_lookup_table(
        &self,
        client: Arc<SolanaRpcClient>,
        lookup_table_address: &Pubkey,
    ) -> Result<(), Box<dyn Error>> {
        let account = client.get_account(lookup_table_address).await?;
        let lookup_table = AddressLookupTable::deserialize(&account.data)?;
        let address_lookup_table_account = AddressLookupTableAccount {
            key: *lookup_table_address,
            addresses: lookup_table.addresses.to_vec(),
        };
        self.add_or_update_table(lookup_table_address.clone(), Some(address_lookup_table_account));
        Ok(())
    }

    /// Add or update address lookup table information - lock-free implementation
    fn add_or_update_table(
        &self,
        lookup_table_address: Pubkey,
        address_lookup_table: Option<AddressLookupTableAccount>,
    ) {
        if let Some(mut entry) = self.tables.get_mut(&lookup_table_address) {
            // Update existing table
            if let Some(table) = address_lookup_table {
                entry.address_lookup_table = Some(table);
            }
        } else {
            // Add new table
            self.tables.insert(
                lookup_table_address,
                AddressLookupTableInfo {
                    lookup_table_address: Some(lookup_table_address),
                    address_lookup_table,
                },
            );
        }
    }

    /// Get table content - high-performance lock-free implementation
    fn get_table_content(&self, lookup_table_address: &Pubkey) -> AddressLookupTableAccount {
        let result = self
            .tables
            .get(lookup_table_address)
            .and_then(|entry| entry.address_lookup_table.clone())
            .unwrap_or_else(|| AddressLookupTableAccount {
                key: *lookup_table_address,
                addresses: Vec::new(),
            });
        return result;
    }
}

/// Get address lookup table account
pub async fn get_address_lookup_table_account(
    lookup_table_address: &Pubkey,
) -> AddressLookupTableAccount {
    let cache = AddressLookupTableCache::get_instance();
    cache.get_table_content(lookup_table_address)
}
