//! Meteora Dynamic Bonding Curve: program accounts, account loading, and the
//! transfer-hook accounts of a transfer-hook pool.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use solana_sdk::{instruction::AccountMeta, pubkey, pubkey::Pubkey};

use crate::{
    common::SolanaRpcClient,
    instruction::utils::meteora_dbc_types::{
        config_decode, pool_decode, DbcConfig, DbcPool, MIGRATION_DAMM_V2,
    },
};

/// Seeds of the program's derived addresses.
pub mod seeds {
    pub const POOL_AUTHORITY_SEED: &[u8] = b"pool_authority";
    pub const EVENT_AUTHORITY_SEED: &[u8] = b"__event_authority";
    /// A transfer hook program's validation account, per mint.
    pub const EXTRA_ACCOUNT_METAS_SEED: &[u8] = b"extra-account-metas";
}

pub mod accounts {
    use solana_sdk::{instruction::AccountMeta, pubkey, pubkey::Pubkey};

    pub const METEORA_DBC: Pubkey = pubkey!("dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN");
    pub const POOL_AUTHORITY: Pubkey = pubkey!("FhVo3mqL8PW5pH5U2CN4XE33DokiyZnUwuGpH2hmHLuM");
    pub const EVENT_AUTHORITY: Pubkey = pubkey!("8Ks12pbrD6PXxfty1hVQiE9sc289zgU1zHkvXhrSdriF");
    pub const SYSVAR_INSTRUCTIONS: Pubkey = pubkey!("Sysvar1nstructions1111111111111111111111111");

    pub const METEORA_DBC_META: AccountMeta =
        AccountMeta { pubkey: METEORA_DBC, is_signer: false, is_writable: false };
    pub const POOL_AUTHORITY_META: AccountMeta =
        AccountMeta { pubkey: POOL_AUTHORITY, is_signer: false, is_writable: false };
    pub const EVENT_AUTHORITY_META: AccountMeta =
        AccountMeta { pubkey: EVENT_AUTHORITY, is_signer: false, is_writable: false };
    pub const SYSVAR_INSTRUCTIONS_META: AccountMeta =
        AccountMeta { pubkey: SYSVAR_INSTRUCTIONS, is_signer: false, is_writable: false };
}

pub const SWAP2_DISCRIMINATOR: [u8; 8] = [65, 75, 63, 76, 235, 91, 91, 136];
pub const SWAP2_WITH_TRANSFER_HOOK_DISCRIMINATOR: [u8; 8] = [183, 93, 153, 40, 24, 230, 194, 151];
pub const SWAP_MODE_EXACT_IN: u8 = 0;
pub const SWAP_MODE_PARTIAL_FILL: u8 = 1;
pub const SWAP_MODE_EXACT_OUT: u8 = 2;
/// `AccountsType::TransferHookBase`: the slice of remaining accounts a
/// transfer of the base token passes to its hook.
pub const TRANSFER_HOOK_BASE_ACCOUNTS: u8 = 0;

pub async fn fetch_pool(rpc: &SolanaRpcClient, pool: &Pubkey) -> Result<DbcPool> {
    let account = rpc.get_account(pool).await?;
    if account.owner != accounts::METEORA_DBC {
        return Err(anyhow!("Account {pool} is not owned by the Meteora DBC program"));
    }
    pool_decode(&account.data).ok_or_else(|| anyhow!("Account {pool} is not a Meteora DBC pool"))
}

pub async fn fetch_config(rpc: &SolanaRpcClient, config: &Pubkey) -> Result<DbcConfig> {
    let account = rpc.get_account(config).await?;
    if account.owner != accounts::METEORA_DBC {
        return Err(anyhow!("Account {config} is not owned by the Meteora DBC program"));
    }
    config_decode(&account.data)
        .ok_or_else(|| anyhow!("Account {config} is not a Meteora DBC config"))
}

/// The DAMM v2 configs a curve migrates with, by its config's
/// `migration_fee_option`: fixed fees of 0.25%, 0.3%, 1%, 2%, 4% and 6%, then
/// the one for a config's own fee.
pub const DAMM_V2_MIGRATION_CONFIGS: [Pubkey; 7] = [
    pubkey!("7F6dnUcRuyM2TwR8myT1dYypFXpPSxqwKNSFNkxyNESd"),
    pubkey!("2nHK1kju6XjphBLbNxpM5XRGFj7p9U8vvNzyZiha1z6k"),
    pubkey!("Hv8Lmzmnju6m7kcokVKvwqz7QPmdX9XfKjJsXz8RXcjp"),
    pubkey!("2c4cYd4reUYVRAB9kUUkrq55VPyy2FNQ3FDL4o12JXmq"),
    pubkey!("AkmQWebAwFvWk55wBoCr5D62C6VVDTzi84NJuD9H7cFD"),
    pubkey!("DbCRBj8McvPYHJG1ukj8RE15h2dCNUdTAESG49XpQ44u"),
    pubkey!("A8gMrEPJkacWkcb3DGwtJwTe16HktSEfvwtuDh2MCtck"),
];

/// The DAMM v2 pool a completed curve of `base_mint` migrates to; `None`
/// for a config that migrates elsewhere. The pool holds the base token as
/// token A and the quote as token B.
pub fn get_migrated_damm_v2_pool(config: &DbcConfig, base_mint: &Pubkey) -> Option<Pubkey> {
    if config.migration_option != MIGRATION_DAMM_V2 {
        return None;
    }
    let damm_config = DAMM_V2_MIGRATION_CONFIGS.get(usize::from(config.migration_fee_option))?;
    // The pool's address orders its mints by their bytes.
    let (first, second) = if base_mint.to_bytes() > config.quote_mint.to_bytes() {
        (base_mint, &config.quote_mint)
    } else {
        (&config.quote_mint, base_mint)
    };
    Some(
        Pubkey::find_program_address(
            &[b"pool", damm_config.as_ref(), first.as_ref(), second.as_ref()],
            &crate::instruction::utils::meteora_damm_v2::accounts::METEORA_DAMM_V2,
        )
        .0,
    )
}

static CONFIGS: Lazy<DashMap<Pubkey, Arc<DbcConfig>>> = Lazy::new(DashMap::new);

/// A config read before, if any. The program has no instruction that changes
/// a config once it is created, so one read serves every pool of the config.
pub fn cached_config(config: &Pubkey) -> Option<Arc<DbcConfig>> {
    CONFIGS.get(config).map(|entry| entry.value().clone())
}

/// Keeps `value` as the config at `config`.
pub fn cache_config(config: Pubkey, value: DbcConfig) -> Arc<DbcConfig> {
    let value = Arc::new(value);
    CONFIGS.insert(config, value.clone());
    value
}

/// [`fetch_config`], read once per config.
pub async fn fetch_config_cached(rpc: &SolanaRpcClient, config: &Pubkey) -> Result<Arc<DbcConfig>> {
    if let Some(cached) = cached_config(config) {
        return Ok(cached);
    }
    Ok(cache_config(*config, fetch_config(rpc, config).await?))
}

// ---------------------------------------------------------------------------
// Transfer hooks
// ---------------------------------------------------------------------------

/// `Execute` of the transfer-hook interface, which tags a validation
/// account's list of extra accounts.
const EXECUTE_DISCRIMINATOR: [u8; 8] = [105, 37, 101, 197, 75, 251, 102, 26];
const EXTRA_ACCOUNT_META_LEN: usize = 35;
/// Accounts `Execute` always takes: source, mint, destination, authority and
/// the validation account. Extra accounts follow.
const EXECUTE_FIXED_ACCOUNTS: usize = 5;

/// An extra account a transfer hook's `Execute` takes
/// (`spl_tlv_account_resolution::account::ExtraAccountMeta`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtraAccountMeta {
    /// 0: `address_config` is the address. 1: a PDA of the hook program, from
    /// the seeds packed in `address_config`. 2: an address read from account
    /// or instruction data. 128 and up: a PDA of the program at `Execute`
    /// account index `discriminator - 128`.
    pub discriminator: u8,
    pub address_config: [u8; 32],
    pub is_signer: bool,
    pub is_writable: bool,
}

/// A hook program's validation account for `mint`.
pub fn get_extra_account_metas_address(mint: &Pubkey, hook_program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[seeds::EXTRA_ACCOUNT_METAS_SEED, mint.as_ref()], hook_program).0
}

/// The extra accounts a validation account lists for `Execute`; `None` when
/// the data is not such a list.
pub fn extra_account_metas_decode(data: &[u8]) -> Option<Vec<ExtraAccountMeta>> {
    // Type-length-value entries: an 8-byte discriminator, a u32 length, the value.
    let mut offset = 0;
    while offset + 12 <= data.len() {
        let discriminator = &data[offset..offset + 8];
        let length = u32::from_le_bytes(data[offset + 8..offset + 12].try_into().ok()?) as usize;
        let value = data.get(offset + 12..offset + 12 + length)?;
        if discriminator == EXECUTE_DISCRIMINATOR {
            let count = u32::from_le_bytes(value.get(..4)?.try_into().ok()?) as usize;
            let metas = value.get(4..4 + count.checked_mul(EXTRA_ACCOUNT_META_LEN)?)?;
            return Some(
                metas
                    .as_chunks::<EXTRA_ACCOUNT_META_LEN>()
                    .0
                    .iter()
                    .map(|meta| ExtraAccountMeta {
                        discriminator: meta[0],
                        address_config: meta[1..33].try_into().expect("32 bytes"),
                        is_signer: meta[33] != 0,
                        is_writable: meta[34] != 0,
                    })
                    .collect(),
            );
        }
        offset += 12 + length;
    }
    None
}

/// Reads the extra accounts `hook_program` takes for transfers of `mint`.
pub async fn fetch_extra_account_metas(
    rpc: &SolanaRpcClient,
    mint: &Pubkey,
    hook_program: &Pubkey,
) -> Result<Vec<ExtraAccountMeta>> {
    let validation = get_extra_account_metas_address(mint, hook_program);
    let account = rpc.get_account(&validation).await?;
    extra_account_metas_decode(&account.data).ok_or_else(|| {
        anyhow!("Account {validation} is not the transfer hook's extra account list of {mint}")
    })
}

/// The seeds packed in an extra account's `address_config`, resolved against
/// the `Execute` accounts known so far.
fn resolve_seeds(address_config: &[u8; 32], accounts: &[Pubkey]) -> Result<Vec<Vec<u8>>> {
    let mut seeds = Vec::new();
    let mut offset = 0;
    while offset < address_config.len() {
        match address_config[offset] {
            0 => break,
            // Literal: length, bytes.
            1 => {
                let length = usize::from(*address_config.get(offset + 1).unwrap_or(&0));
                let bytes = address_config
                    .get(offset + 2..offset + 2 + length)
                    .ok_or_else(|| anyhow!("Transfer hook seed literal is cut short"))?;
                seeds.push(bytes.to_vec());
                offset += 2 + length;
            }
            // Account key: index into the `Execute` accounts.
            3 => {
                let index = usize::from(*address_config.get(offset + 1).unwrap_or(&u8::MAX));
                let key = accounts.get(index).ok_or_else(|| {
                    anyhow!("Transfer hook seed names account {index}, not known yet")
                })?;
                seeds.push(key.to_bytes().to_vec());
                offset += 2;
            }
            // Instruction data (the transfer's amount) and account data are
            // not known when a swap is built.
            2 => {
                return Err(anyhow!("Transfer hook seeds from instruction data are not supported"))
            }
            4 => return Err(anyhow!("Transfer hook seeds from account data are not supported")),
            other => return Err(anyhow!("Unknown transfer hook seed type {other}")),
        }
    }
    Ok(seeds)
}

/// The accounts a transfer of `mint` from `source` to `destination` by
/// `authority` passes to its hook: the hook's extra accounts in their listed
/// order, then the hook program and its validation account.
///
/// Extra accounts are fixed addresses or addresses derived from literals and
/// the transfer's accounts; a hook whose accounts depend on instruction or
/// account data, or which needs another signer, is an error.
pub fn resolve_transfer_hook_accounts(
    hook_program: &Pubkey,
    mint: &Pubkey,
    metas: &[ExtraAccountMeta],
    source: &Pubkey,
    destination: &Pubkey,
    authority: &Pubkey,
) -> Result<Vec<AccountMeta>> {
    let validation = get_extra_account_metas_address(mint, hook_program);
    let mut execute_accounts = Vec::with_capacity(EXECUTE_FIXED_ACCOUNTS + metas.len());
    execute_accounts.extend([*source, *mint, *destination, *authority, validation]);
    let mut resolved = Vec::with_capacity(metas.len() + 2);
    for meta in metas {
        if meta.is_signer {
            return Err(anyhow!("Transfer hook of {mint} needs another signer"));
        }
        let pubkey = match meta.discriminator {
            0 => Pubkey::new_from_array(meta.address_config),
            1 => derive(hook_program, &resolve_seeds(&meta.address_config, &execute_accounts)?),
            2 => {
                return Err(anyhow!(
                    "Transfer hook accounts read from account or instruction data are not supported"
                ))
            }
            index @ 128.. => {
                let program = *execute_accounts.get(usize::from(index - 128)).ok_or_else(|| {
                    anyhow!("Transfer hook account derives from a program not known yet")
                })?;
                derive(&program, &resolve_seeds(&meta.address_config, &execute_accounts)?)
            }
            other => return Err(anyhow!("Unknown transfer hook account type {other}")),
        };
        execute_accounts.push(pubkey);
        resolved.push(AccountMeta { pubkey, is_signer: false, is_writable: meta.is_writable });
    }
    resolved.push(AccountMeta::new_readonly(*hook_program, false));
    resolved.push(AccountMeta::new_readonly(validation, false));
    Ok(resolved)
}

fn derive(program: &Pubkey, seeds: &[Vec<u8>]) -> Pubkey {
    let seeds: Vec<&[u8]> = seeds.iter().map(Vec::as_slice).collect();
    Pubkey::find_program_address(&seeds, program).0
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey;

    #[test]
    fn program_addresses_are_the_derived_ones() {
        assert_eq!(
            Pubkey::find_program_address(&[seeds::POOL_AUTHORITY_SEED], &accounts::METEORA_DBC).0,
            accounts::POOL_AUTHORITY
        );
        assert_eq!(
            Pubkey::find_program_address(&[seeds::EVENT_AUTHORITY_SEED], &accounts::METEORA_DBC).0,
            accounts::EVENT_AUTHORITY
        );
    }

    #[test]
    fn migrated_pool_is_derived_from_the_configs_fee_option() {
        // Mainnet pool 2Rz8zRLAqMtXKBGsxb8DwYN1Ed13TDwLtxNUrEUHtBJY (USDC quote,
        // its own fee) migrated to EPy3Rnwz9G1eg1wx6a9wCoEsnSFCwb3r4keFFzxauLLX.
        let mut config = DbcConfig {
            quote_mint: pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"),
            migration_option: MIGRATION_DAMM_V2,
            migration_fee_option: 6,
            ..Default::default()
        };
        let base_mint = pubkey!("mAo7GAjCZ2LCW5yNoQW31Ce9kLttUMCjyjD2kP9ever");
        assert_eq!(
            get_migrated_damm_v2_pool(&config, &base_mint),
            Some(pubkey!("EPy3Rnwz9G1eg1wx6a9wCoEsnSFCwb3r4keFFzxauLLX"))
        );
        // Another fee option is another pool; DAMM v1 and unknown options none.
        config.migration_fee_option = 3;
        assert_ne!(
            get_migrated_damm_v2_pool(&config, &base_mint),
            Some(pubkey!("EPy3Rnwz9G1eg1wx6a9wCoEsnSFCwb3r4keFFzxauLLX"))
        );
        config.migration_fee_option = 7;
        assert_eq!(get_migrated_damm_v2_pool(&config, &base_mint), None);
        config.migration_fee_option = 6;
        config.migration_option = 0;
        assert_eq!(get_migrated_damm_v2_pool(&config, &base_mint), None);
    }

    fn meta(discriminator: u8, config: &[u8], is_writable: bool) -> ExtraAccountMeta {
        let mut address_config = [0u8; 32];
        address_config[..config.len()].copy_from_slice(config);
        ExtraAccountMeta { discriminator, address_config, is_signer: false, is_writable }
    }

    fn validation_data(metas: &[ExtraAccountMeta]) -> Vec<u8> {
        let mut value = (metas.len() as u32).to_le_bytes().to_vec();
        for meta in metas {
            value.push(meta.discriminator);
            value.extend_from_slice(&meta.address_config);
            value.push(u8::from(meta.is_signer));
            value.push(u8::from(meta.is_writable));
        }
        let mut data = EXECUTE_DISCRIMINATOR.to_vec();
        data.extend_from_slice(&(value.len() as u32).to_le_bytes());
        data.extend_from_slice(&value);
        data
    }

    /// PDA of the hook program from a literal and the mint (account 1).
    fn literal_and_mint(literal: &[u8]) -> Vec<u8> {
        let mut config = vec![1, literal.len() as u8];
        config.extend_from_slice(literal);
        config.extend_from_slice(&[3, 1]);
        config
    }

    #[test]
    fn validation_account_round_trips_and_other_entries_are_skipped() {
        let metas = vec![
            meta(1, &literal_and_mint(b"cfg"), true),
            meta(0, accounts::SYSVAR_INSTRUCTIONS.as_ref(), false),
        ];
        assert_eq!(extra_account_metas_decode(&validation_data(&metas)), Some(metas.clone()));

        // An entry of another instruction ahead of the Execute one.
        let mut data = vec![9u8; 8];
        data.extend_from_slice(&3u32.to_le_bytes());
        data.extend_from_slice(&[1, 2, 3]);
        data.extend_from_slice(&validation_data(&metas));
        assert_eq!(extra_account_metas_decode(&data), Some(metas));

        assert_eq!(extra_account_metas_decode(&validation_data(&[])), Some(Vec::new()));
        assert_eq!(extra_account_metas_decode(&[0u8; 7]), None);
        let mut cut = validation_data(&[meta(0, &[7; 32], false)]);
        cut.truncate(cut.len() - 1);
        assert_eq!(extra_account_metas_decode(&cut), None);
    }

    /// The hooks of mainnet transfer-hook pools, with the accounts their swaps
    /// passed.
    #[test]
    fn mainnet_hooks_resolve_to_the_accounts_their_swaps_passed() {
        let source = Pubkey::new_unique();
        let destination = Pubkey::new_unique();
        let authority = Pubkey::new_unique();

        // No extra accounts: the program and its validation account alone.
        let hook = pubkey!("887b3SjuJv9t9wP6fd7Fe7dqFhksC8c39a2PJRa3cGec");
        let mint = pubkey!("mAo7GAjCZ2LCW5yNoQW31Ce9kLttUMCjyjD2kP9ever");
        let accounts =
            resolve_transfer_hook_accounts(&hook, &mint, &[], &source, &destination, &authority)
                .unwrap();
        assert_eq!(
            accounts,
            vec![
                AccountMeta::new_readonly(hook, false),
                AccountMeta::new_readonly(
                    pubkey!("FGZZEin9TMPnyNMPvdXtgNRPD41f8gV6sByTnDVRXiUR"),
                    false
                ),
            ]
        );

        // PDA(["cfg", mint]) writable, the instructions sysvar, PDA(["ext", mint]).
        let hook = pubkey!("C3vEdPepTPRrJdQ4nQ3ZmhdXCmpKdGRKVUqxduHZWbdR");
        let mint = pubkey!("HAoowFkDyWfuetaV7DBdmU4aesB5jn8jL7uRpnSWengW");
        let metas = [
            meta(1, &literal_and_mint(b"cfg"), true),
            meta(0, super::accounts::SYSVAR_INSTRUCTIONS.as_ref(), false),
            meta(1, &literal_and_mint(b"ext"), false),
        ];
        let accounts =
            resolve_transfer_hook_accounts(&hook, &mint, &metas, &source, &destination, &authority)
                .unwrap();
        assert_eq!(
            accounts,
            vec![
                AccountMeta::new(pubkey!("GjucFNkLjTEEMmyxfaR73273Fohb32CHuLDY5A2D4u62"), false),
                AccountMeta::new_readonly(super::accounts::SYSVAR_INSTRUCTIONS, false),
                AccountMeta::new_readonly(
                    pubkey!("C6oQ6WxUUDu5qWikbHXqjuwSEwk8oKSyXNz6AQBBTRCn"),
                    false
                ),
                AccountMeta::new_readonly(hook, false),
                AccountMeta::new_readonly(
                    pubkey!("dipY1shpTXvQtAp3LwBhYVEZndJfP7ZAS6sbSH61Sei"),
                    false
                ),
            ]
        );
    }

    #[test]
    fn hooks_this_cannot_resolve_are_errors() {
        let key = Pubkey::new_unique();
        let resolve = |metas: &[ExtraAccountMeta]| {
            resolve_transfer_hook_accounts(&key, &key, metas, &key, &key, &key)
        };
        // Seeds from account data, from instruction data, an address read
        // from data, and a second signer.
        assert!(resolve(&[meta(1, &[4, 2, 32, 32], false)]).is_err());
        assert!(resolve(&[meta(1, &[2, 8, 8], false)]).is_err());
        assert!(resolve(&[meta(2, &[1, 8], false)]).is_err());
        let mut signer = meta(0, &[1; 32], false);
        signer.is_signer = true;
        assert!(resolve(&[signer]).is_err());
        // A seed naming an extra account that comes later.
        assert!(resolve(&[meta(1, &[3, 6], false)]).is_err());
        // A PDA of an earlier account's program resolves.
        assert!(resolve(&[meta(128 + 1, &literal_and_mint(b"x"), false)]).is_ok());
    }
}
