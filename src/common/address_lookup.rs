use crate::common::SolanaRpcClient;
use anyhow::Result;
use solana_address_lookup_table_interface::state::AddressLookupTable;
use solana_sdk::{message::AddressLookupTableAccount, pubkey::Pubkey};

pub async fn fetch_address_lookup_table_account(
    rpc: &SolanaRpcClient,
    lookup_table_address: &Pubkey,
) -> Result<AddressLookupTableAccount, anyhow::Error> {
    let account = rpc.get_account(lookup_table_address).await?;
    let lookup_table = AddressLookupTable::deserialize(&account.data)?;
    let address_lookup_table_account = AddressLookupTableAccount {
        key: *lookup_table_address,
        addresses: lookup_table.addresses.to_vec(),
    };
    Ok(address_lookup_table_account)
}
