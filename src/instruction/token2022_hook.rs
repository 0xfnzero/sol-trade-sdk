//! Offline Hook resolver for literals and SPL PDA seeds.
//! Other configurations fail closed; refresh mint and TLV data per transfer.
use anyhow::{bail, ensure, Result};
use solana_sdk::{instruction::AccountMeta, pubkey::Pubkey};

pub fn resolve_hook_accounts(
    hook: &Pubkey,
    mint: &Pubkey,
    mint_owner: &Pubkey,
    mint_data: &[u8],
    meta: &Pubkey,
    meta_owner: &Pubkey,
    meta_data: &[u8],
    execute_accounts: &[Pubkey],
) -> Result<Vec<AccountMeta>> {
    resolve_hook_accounts_with_context(
        hook,
        mint,
        mint_owner,
        mint_data,
        meta,
        meta_owner,
        meta_data,
        execute_accounts,
        &[],
        &std::collections::HashMap::new(),
    )
}

/// Resolve SPL PDA seeds using fresh Execute data and callback account snapshots.
/// Signer metadata remains unsupported.
pub fn resolve_hook_accounts_with_context(
    hook: &Pubkey,
    mint: &Pubkey,
    mint_owner: &Pubkey,
    mint_data: &[u8],
    meta: &Pubkey,
    meta_owner: &Pubkey,
    meta_data: &[u8],
    execute_accounts: &[Pubkey],
    execute_data: &[u8],
    account_data: &std::collections::HashMap<Pubkey, Vec<u8>>,
) -> Result<Vec<AccountMeta>> {
    ensure!(
        execute_data.is_empty()
            || (execute_data.len() == 16
                && execute_data[..8] == [105, 37, 101, 197, 75, 251, 102, 26]),
        "Invalid Execute instruction data"
    );
    ensure!(
        *mint_owner == spl_token_2022_interface::id()
            && mint_data.len() >= 166
            && mint_data[165] == 1
            && mint_data[45] == 1,
        "Invalid Token-2022 mint"
    );
    let mut active = None;
    let mut offset = 166;
    while offset + 4 <= mint_data.len() {
        let kind = u16::from_le_bytes(mint_data[offset..offset + 2].try_into()?);
        let length = u16::from_le_bytes(mint_data[offset + 2..offset + 4].try_into()?) as usize;
        let end = offset + 4 + length;
        ensure!(end <= mint_data.len(), "Truncated mint extension");
        if kind == 14 {
            ensure!(active.is_none() && length == 64, "Invalid Hook extension");
            active = Some(Pubkey::new_from_array(mint_data[offset + 36..end].try_into()?));
        }
        offset = end;
    }
    ensure!(active == Some(*hook) && *hook != Pubkey::default(), "Active Hook program mismatch");
    let expected = Pubkey::find_program_address(&[b"extra-account-metas", mint.as_ref()], hook).0;
    ensure!(*meta == expected && meta_owner == hook, "Invalid Hook validation account");
    ensure!(
        execute_accounts.len() == 5 && execute_accounts[1] == *mint && execute_accounts[4] == *meta,
        "Invalid Execute account order"
    );
    ensure!(
        meta_data.len() >= 16 && meta_data[..8] == [105, 37, 101, 197, 75, 251, 102, 26],
        "Invalid Execute TLV"
    );
    let count = u32::from_le_bytes(meta_data[12..16].try_into()?) as u64;
    ensure!(
        u32::from_le_bytes(meta_data[8..12].try_into()?) as u64 == 4 + 35 * count
            && meta_data.len() as u64 == 16 + 35 * count,
        "Invalid Execute TLV length"
    );
    let mut keys = execute_accounts.to_vec();
    let mut result: Vec<AccountMeta> = Vec::new();
    for item in meta_data[16..].chunks_exact(35) {
        ensure!(item[33] == 0 && item[34] <= 1, "Unsupported signer or invalid flags");
        let config = &item[1..33];
        let key = match item[0] {
            0 => Pubkey::new_from_array(config.try_into()?),
            2 => {
                let data: &[u8];
                let start: usize;
                match config[0] {
                    1 => {
                        start = config[1] as usize;
                        ensure!(
                            config[2..].iter().all(|b| *b == 0),
                            "Invalid instruction PubkeyData"
                        );
                        data = execute_data;
                    }
                    2 => {
                        start = config[2] as usize;
                        ensure!(config[3..].iter().all(|b| *b == 0), "Invalid account PubkeyData");
                        let account = keys
                            .get(config[1] as usize)
                            .ok_or_else(|| anyhow::anyhow!("Invalid account PubkeyData"))?;
                        data = account_data
                            .get(account)
                            .ok_or_else(|| anyhow::anyhow!("Missing PubkeyData snapshot"))?;
                    }
                    _ => bail!("Invalid PubkeyData configuration"),
                }
                ensure!(start + 32 <= data.len(), "Invalid PubkeyData range");
                Pubkey::new_from_array(data[start..start + 32].try_into()?)
            }
            tag if tag == 1 || tag >= 128 => {
                let program = if tag == 1 {
                    *hook
                } else {
                    *keys
                        .get((tag - 128) as usize)
                        .ok_or_else(|| anyhow::anyhow!("Invalid external Hook PDA program index"))?
                };
                let mut owned: Vec<Vec<u8>> = Vec::new();
                let mut offset = 0;
                while offset < 32 && config[offset] != 0 {
                    match config[offset] {
                        1 => {
                            ensure!(offset + 1 < 32, "Truncated literal Hook PDA seed");
                            let length = config[offset + 1] as usize;
                            ensure!(
                                length <= 32 && offset + 2 + length <= 32,
                                "Invalid literal Hook PDA seed"
                            );
                            owned.push(config[offset + 2..offset + 2 + length].to_vec());
                            offset += 2 + length;
                        }
                        2 => {
                            ensure!(offset + 2 < 32, "Truncated instruction Hook PDA seed");
                            let start = config[offset + 1] as usize;
                            let length = config[offset + 2] as usize;
                            ensure!(
                                !execute_data.is_empty()
                                    && length <= 32
                                    && start + length <= execute_data.len(),
                                "Missing or invalid Execute seed data"
                            );
                            owned.push(execute_data[start..start + length].to_vec());
                            offset += 3;
                        }
                        3 => {
                            ensure!(
                                offset + 1 < 32 && (config[offset + 1] as usize) < keys.len(),
                                "Invalid account-key Hook PDA seed"
                            );
                            owned.push(keys[config[offset + 1] as usize].to_bytes().to_vec());
                            offset += 2;
                        }
                        4 => {
                            ensure!(
                                offset + 3 < 32 && (config[offset + 1] as usize) < keys.len(),
                                "Invalid account-data Hook PDA seed"
                            );
                            let start = config[offset + 2] as usize;
                            let length = config[offset + 3] as usize;
                            let data = account_data
                                .get(&keys[config[offset + 1] as usize])
                                .ok_or_else(|| anyhow::anyhow!("Missing account seed data"))?;
                            ensure!(
                                length <= 32 && start + length <= data.len(),
                                "Invalid account seed data"
                            );
                            owned.push(data[start..start + length].to_vec());
                            offset += 4;
                        }
                        _ => bail!("Unsupported Hook PDA seed"),
                    }
                }
                ensure!(
                    config[offset..].iter().all(|b| *b == 0) && owned.len() <= 15,
                    "Invalid Hook PDA seed padding/count"
                );
                let seeds: Vec<&[u8]> = owned.iter().map(Vec::as_slice).collect();
                Pubkey::try_find_program_address(&seeds, &program)
                    .ok_or_else(|| anyhow::anyhow!("Invalid Hook PDA"))?
                    .0
            }
            _ => bail!("Unsupported Hook account configuration"),
        };
        let duplicates: Vec<_> = result.iter().filter(|a| a.pubkey == key).collect();
        let writable = item[34] != 0
            && !execute_accounts.contains(&key)
            && (duplicates.is_empty() || duplicates.iter().any(|a| a.is_writable));
        result.push(AccountMeta { pubkey: key, is_signer: false, is_writable: writable });
        keys.push(key);
    }
    result.push(AccountMeta::new_readonly(*hook, false));
    result.push(AccountMeta::new_readonly(*meta, false));
    Ok(result)
}
