use crate::common::SolanaRpcClient;
use anyhow::Result;
use solana_address_lookup_table_interface::state::AddressLookupTable;
use solana_sdk::{
    message::{v0, AddressLookupTableAccount},
    pubkey::Pubkey,
};

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

#[inline]
pub fn extract_lookup_table_indexes(
    instructions: &[solana_sdk::instruction::Instruction],
    lookup_table_account: &AddressLookupTableAccount,
) -> Option<v0::MessageAddressTableLookup> {
    use std::collections::{HashMap, HashSet};

    // 构建地址到索引的映射（O(1) 查找）
    let addr_to_index: HashMap<&Pubkey, u8> = lookup_table_account
        .addresses
        .iter()
        .enumerate()
        .filter_map(|(idx, addr)| u8::try_from(idx).ok().map(|i| (addr, i)))
        .collect();

    // 收集所有需要的账户及其权限
    let mut writable_indexes = Vec::new();
    let mut readonly_indexes = Vec::new();
    let mut seen = HashSet::new();

    for instruction in instructions {
        for account_meta in &instruction.accounts {
            // 跳过已处理的账户
            if !seen.insert(&account_meta.pubkey) {
                continue;
            }

            // 在查找表中查找账户
            if let Some(&index) = addr_to_index.get(&account_meta.pubkey) {
                if account_meta.is_writable {
                    writable_indexes.push(index);
                } else {
                    readonly_indexes.push(index);
                }
            }
        }
    }

    // 如果没有找到任何账户，返回 None
    if writable_indexes.is_empty() && readonly_indexes.is_empty() {
        return None;
    }

    Some(v0::MessageAddressTableLookup {
        account_key: lookup_table_account.key,
        writable_indexes,
        readonly_indexes,
    })
}
