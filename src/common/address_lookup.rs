use crate::common::SolanaRpcClient;
use solana_message::AddressLookupTableAccount;
use solana_sdk::pubkey::Pubkey;

const LOOKUP_TABLE_PROGRAM: Pubkey =
    solana_sdk::pubkey!("AddressLookupTab1e1111111111111111111111111");

pub async fn fetch_address_lookup_table_account(
    rpc: &SolanaRpcClient,
    lookup_table_address: &Pubkey,
) -> Result<AddressLookupTableAccount, anyhow::Error> {
    let account = rpc.get_account(lookup_table_address).await?;
    if account.owner != LOOKUP_TABLE_PROGRAM {
        anyhow::bail!("Account is not owned by the address lookup table program");
    }
    decode_address_lookup_table(lookup_table_address, &account.data)
}

/// Decode a subscription-fed ALT account without RPC. Validate owner before
/// interpreting bytes; preserve every address including the System Program.
pub fn decode_address_lookup_table_account(
    key: &Pubkey,
    owner: &Pubkey,
    data: &[u8],
) -> anyhow::Result<AddressLookupTableAccount> {
    if *owner != LOOKUP_TABLE_PROGRAM {
        anyhow::bail!("Account is not owned by the address lookup table program");
    }
    decode_address_lookup_table(key, data)
}

fn decode_address_lookup_table(
    key: &Pubkey,
    data: &[u8],
) -> anyhow::Result<AddressLookupTableAccount> {
    // Initialized ProgramState::LookupTable is bincode tag 1; metadata is 56 bytes.
    if data.len() < 56 || data[..4] != [1, 0, 0, 0] {
        anyhow::bail!("Invalid initialized address lookup table metadata");
    }
    let tail = &data[56..];
    if tail.len() % 32 != 0 || tail.len() / 32 > 256 {
        anyhow::bail!("Invalid address lookup table address array");
    }
    // Every slot is meaningful. The all-zero pubkey is the System Program,
    // not unused padding. Filtering it shifts all subsequent on-chain indices.
    let addresses = tail
        .chunks_exact(32)
        .map(|bytes| Pubkey::new_from_array(bytes.try_into().expect("32-byte chunk")))
        .collect();
    Ok(AddressLookupTableAccount { key: *key, addresses })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn system_program_entries_preserve_every_lookup_index() {
        let first = Pubkey::new_unique();
        let last = Pubkey::new_unique();
        let mut data = vec![0; 56];
        data[..4].copy_from_slice(&[1, 0, 0, 0]);
        for address in [first, Pubkey::default(), last] {
            data.extend_from_slice(address.as_ref());
        }
        let table = decode_address_lookup_table(&Pubkey::new_unique(), &data).unwrap();
        assert_eq!(table.addresses, vec![first, Pubkey::default(), last]);
        assert_eq!(table.addresses[2], last);
    }
    #[test]
    fn subscription_adapter_validates_owner_and_preserves_zero_keys() {
        let key = Pubkey::new_unique();
        let mut data = vec![0; 88];
        data[0] = 1;
        assert!(decode_address_lookup_table_account(&key, &Pubkey::new_unique(), &data).is_err());
        let table =
            decode_address_lookup_table_account(&key, &LOOKUP_TABLE_PROGRAM, &data).unwrap();
        assert_eq!(table.addresses, vec![Pubkey::default()]);
    }
    #[test]
    fn rejects_uninitialized_and_partial_lookup_tables() {
        assert!(decode_address_lookup_table(&Pubkey::new_unique(), &[0; 56]).is_err());
        let mut data = vec![0; 57];
        data[0] = 1;
        assert!(decode_address_lookup_table(&Pubkey::new_unique(), &data).is_err());
    }
}
