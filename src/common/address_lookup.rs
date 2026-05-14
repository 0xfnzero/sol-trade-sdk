use crate::common::SolanaRpcClient;
use anyhow::Result;
use solana_message::AddressLookupTableAccount;
use solana_sdk::pubkey::Pubkey;

pub async fn fetch_address_lookup_table_account(
    rpc: &SolanaRpcClient,
    lookup_table_address: &Pubkey,
) -> Result<AddressLookupTableAccount, anyhow::Error> {
    let account = rpc.get_account(lookup_table_address).await?;

    // Parse address lookup table manually
    // Layout: 4 bytes (type) + 4 bytes (deactivation_slot) + 4 bytes (last_extended_slot) + 1 byte (last_extended_slot_start_index) + 1 byte (authority) + padding
    // Then addresses start at offset 56, each address is 32 bytes
    // First 4 bytes indicate if initialized (should be 1 or 2)

    if account.data.len() < 56 {
        return Err(anyhow::anyhow!("Address lookup table account data too short"));
    }

    // Read number of addresses (stored at offset 20 as u32, but we need to scan the bitmap)
    // Actually simpler: addresses start at offset 56, count from bitmap at offset 8-20
    let mut addresses = Vec::new();
    let mut offset = 56;
    while offset + 32 <= account.data.len() {
        let addr_bytes: [u8; 32] = account.data[offset..offset + 32].try_into()?;
        // Skip zero addresses (unused slots)
        if addr_bytes != [0u8; 32] {
            addresses.push(Pubkey::from(addr_bytes));
        }
        offset += 32;
    }

    let address_lookup_table_account =
        AddressLookupTableAccount { key: *lookup_table_address, addresses };
    Ok(address_lookup_table_account)
}
