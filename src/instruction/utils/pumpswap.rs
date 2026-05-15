use crate::{
    common::{
        spl_associated_token_account::get_associated_token_address_with_program_id, SolanaRpcClient,
    },
    constants::{TOKEN_PROGRAM, WSOL_TOKEN_ACCOUNT},
    instruction::utils::pumpswap_types::{pool_decode, Pool},
};
use anyhow::anyhow;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use rand::seq::IndexedRandom;
use solana_account_decoder::UiAccountEncoding;
use solana_sdk::{instruction::AccountMeta, pubkey::Pubkey};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tracing::warn;

// Pool account sizes moved to find_by_base_mint/find_by_quote_mint (POOL_DATA_LEN_SPL, POOL_DATA_LEN_T22)

/// Constants used as seeds for deriving PDAs (Program Derived Addresses)
pub mod seeds {
    /// Seed for the global state PDA
    pub const GLOBAL_SEED: &[u8] = b"global";

    /// Seed for the mint authority PDA
    pub const MINT_AUTHORITY_SEED: &[u8] = b"mint-authority";

    /// Seed for bonding curve PDAs
    pub const BONDING_CURVE_SEED: &[u8] = b"bonding-curve";

    /// Seed for metadata PDAs
    pub const METADATA_SEED: &[u8] = b"metadata";

    pub const USER_VOLUME_ACCUMULATOR_SEED: &[u8] = b"user_volume_accumulator";
    pub const GLOBAL_VOLUME_ACCUMULATOR_SEED: &[u8] = b"global_volume_accumulator";
    pub const FEE_CONFIG_SEED: &[u8] = b"fee_config";

    /// Seed for pool v2 PDA (required by program upgrade, readonly at end of account list)
    pub const POOL_V2_SEED: &[u8] = b"pool-v2";
    /// Legacy pool PDA seed (used with index, creator, base_mint, quote_mint)
    pub const POOL_SEED: &[u8] = b"pool";
    /// Pump program: pool-authority PDA seed (creator for canonical pool)
    pub const POOL_AUTHORITY_SEED: &[u8] = b"pool-authority";
}

/// Constants related to program accounts and authorities
pub mod accounts {
    use solana_sdk::{pubkey, pubkey::Pubkey};

    /// Public key for the fee recipient
    pub const FEE_RECIPIENT: Pubkey = pubkey!("62qc2CNXwrYqQScmEdiZFFAnJR262PxWEuNQtxfafNgV");

    /// Public key for the global PDA
    pub const GLOBAL_ACCOUNT: Pubkey = pubkey!("ADyA8hdefvWN2dbGGWFotbzWxrAvLW83WG6QCVXvJKqw");

    /// Authority for program events
    pub const EVENT_AUTHORITY: Pubkey = pubkey!("GS4CU59F31iL7aR2Q8zVS8DRrcRnXX1yjQ66TqNVQnaR");

    /// Associated Token Program ID
    pub const ASSOCIATED_TOKEN_PROGRAM: Pubkey =
        pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

    // PumpSwap protocol fee recipient
    pub const PROTOCOL_FEE_RECIPIENT: Pubkey =
        pubkey!("62qc2CNXwrYqQScmEdiZFFAnJR262PxWEuNQtxfafNgV");

    pub const AMM_PROGRAM: Pubkey = pubkey!("pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA");
    /// Pump Bonding Curve program（canonical pool 的 creator 来自此程序的 pool-authority PDA）
    pub const PUMP_PROGRAM_ID: Pubkey = pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P");

    pub const LP_FEE_BASIS_POINTS: u64 = 25;
    pub const PROTOCOL_FEE_BASIS_POINTS: u64 = 5;
    pub const COIN_CREATOR_FEE_BASIS_POINTS: u64 = 5;

    pub const FEE_PROGRAM: Pubkey = pubkey!("pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ");

    pub const GLOBAL_VOLUME_ACCUMULATOR: Pubkey =
        pubkey!("C2aFPdENg4A2HQsmrd5rTw5TaYBX5Ku887cWjbFKtZpw"); // get_global_volume_accumulator_pda().unwrap();

    pub const FEE_CONFIG: Pubkey = pubkey!("5PHirr8joyTMp9JMm6nW7hNDVyEYdkzDqazxPD7RaTjx"); // get_fee_config_pda().unwrap();

    pub const DEFAULT_COIN_CREATOR_VAULT_AUTHORITY: Pubkey =
        pubkey!("8N3GDaZ2iwN65oxVatKTLPNooAVUJTbfiVJ1ahyqwjSk");

    /// Mayhem fee recipients (pump-public-docs: use any one randomly for throughput)
    pub const MAYHEM_FEE_RECIPIENTS: [Pubkey; 8] = [
        pubkey!("GesfTA3X2arioaHp8bbKdjG9vJtskViWACZoYvxp4twS"),
        pubkey!("4budycTjhs9fD6xw62VBducVTNgMgJJ5BgtKq7mAZwn6"),
        pubkey!("8SBKzEQU4nLSzcwF4a74F2iaUDQyTfjGndn6qUWBnrpR"),
        pubkey!("4UQeTP1T39KZ9Sfxzo3WR5skgsaP6NZa87BAkuazLEKH"),
        pubkey!("8sNeir4QsLsJdYpc9RZacohhK1Y5FLU3nC5LXgYB4aa6"),
        pubkey!("Fh9HmeLNUMVCvejxCtCL2DbYaRyBFVJ5xrWkLnMH6fdk"),
        pubkey!("463MEnMeGyJekNZFQSTUABBEbLnvMTALbT6ZmsxAbAdq"),
        pubkey!("6AUH3WEHucYZyC61hqpqYUWVto5qA5hjHuNQ32GNnNxA"),
    ];
    /// Default Mayhem fee recipient (first of MAYHEM_FEE_RECIPIENTS)
    pub const MAYHEM_FEE_RECIPIENT: Pubkey = MAYHEM_FEE_RECIPIENTS[0];

    /// Buyback trailing fee recipients (`GlobalConfig.buyback_fee_recipients` on Pump AMM).
    /// Must match one of these for the pubkey passed after optional `pool-v2` (`@pump-fun/pump-swap-sdk` `getBuybackFeeRecipient`).
    /// Static mirror of pump-public-docs; if protocol rotates configs, decode global_config from RPC.
    pub const PROTOCOL_EXTRA_FEE_RECIPIENTS: [Pubkey; 8] = [
        pubkey!("5YxQFdt3Tr9zJLvkFccqXVUwhdTWJQc1fFg2YPbxvxeD"),
        pubkey!("9M4giFFMxmFGXtc3feFzRai56WbBqehoSeRE5GK7gf7"),
        pubkey!("GXPFM2caqTtQYC2cJ5yJRi9VDkpsYZXzYdwYpGnLmtDL"),
        pubkey!("3BpXnfJaUTiwXnJNe7Ej1rcbzqTTQUvLShZaWazebsVR"),
        pubkey!("5cjcW9wExnJJiqgLjq7DEG75Pm6JBgE1hNv4B2vHXUW6"),
        pubkey!("EHAAiTxcdDwQ3U4bU6YcMsQGaekdzLS3B5SmYo46kJtL"),
        pubkey!("5eHhjP8JaYkz83CWwvGU2uMUXefd3AazWGx4gpcuEEYD"),
        pubkey!("A7hAgCzFw14fejgCp387JUJRMNyz4j89JKnhtKU8piqW"),
    ];

    // META

    pub const GLOBAL_ACCOUNT_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: GLOBAL_ACCOUNT,
            is_signer: false,
            is_writable: false,
        };

    pub const FEE_RECIPIENT_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: FEE_RECIPIENT,
            is_signer: false,
            is_writable: false,
        };

    pub const MAYHEM_FEE_RECIPIENT_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: MAYHEM_FEE_RECIPIENT,
            is_signer: false,
            is_writable: false,
        };

    pub const ASSOCIATED_TOKEN_PROGRAM_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: ASSOCIATED_TOKEN_PROGRAM,
            is_signer: false,
            is_writable: false,
        };

    pub const EVENT_AUTHORITY_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: EVENT_AUTHORITY,
            is_signer: false,
            is_writable: false,
        };

    pub const AMM_PROGRAM_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: AMM_PROGRAM,
            is_signer: false,
            is_writable: false,
        };

    pub const GLOBAL_VOLUME_ACCUMULATOR_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: GLOBAL_VOLUME_ACCUMULATOR,
            is_signer: false,
            is_writable: true,
        };

    pub const FEE_CONFIG_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: FEE_CONFIG,
            is_signer: false,
            is_writable: false,
        };

    pub const FEE_PROGRAM_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: FEE_PROGRAM,
            is_signer: false,
            is_writable: false,
        };
}

pub const BUY_DISCRIMINATOR: [u8; 8] = [102, 6, 61, 18, 1, 218, 235, 234];
pub const BUY_EXACT_QUOTE_IN_DISCRIMINATOR: [u8; 8] = [198, 46, 21, 82, 180, 217, 232, 112];
pub const SELL_DISCRIMINATOR: [u8; 8] = [51, 230, 133, 164, 1, 127, 131, 173];

const PUMPSWAP_GLOBAL_CONFIG_TTL: Duration = Duration::from_secs(90);
const PUMPSWAP_GLOBAL_CONFIG_RPC_TIMEOUT: Duration = Duration::from_millis(180);

const PUBKEY_LEN: usize = 32;
const U64_LEN: usize = 8;
const U8_LEN: usize = 1;
const BOOL_LEN: usize = 1;
const GLOBAL_CONFIG_DISCRIMINATOR_LEN: usize = 8;

#[derive(Clone, Debug)]
pub struct GlobalConfig {
    pub protocol_fee_recipients: [Pubkey; 8],
    pub reserved_fee_recipient: Pubkey,
    pub reserved_fee_recipients: [Pubkey; 7],
    pub buyback_fee_recipients: [Pubkey; 8],
}

#[derive(Clone)]
struct CachedGlobalConfig {
    fetched_at: Instant,
    config: GlobalConfig,
}

static GLOBAL_CONFIG_CACHE: Lazy<RwLock<Option<CachedGlobalConfig>>> =
    Lazy::new(|| RwLock::new(None));
static GLOBAL_CONFIG_REFRESH_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

fn read_pubkey(data: &[u8], offset: usize) -> Option<Pubkey> {
    let bytes = data.get(offset..offset + PUBKEY_LEN)?;
    Some(Pubkey::new_from_array(bytes.try_into().ok()?))
}

fn read_pubkey_array<const N: usize>(data: &[u8], offset: usize) -> Option<[Pubkey; N]> {
    let mut keys = [Pubkey::default(); N];
    for (i, key) in keys.iter_mut().enumerate() {
        *key = read_pubkey(data, offset + i * PUBKEY_LEN)?;
    }
    Some(keys)
}

fn decode_global_config(data: &[u8]) -> Option<GlobalConfig> {
    let mut offset = GLOBAL_CONFIG_DISCRIMINATOR_LEN;
    offset += PUBKEY_LEN; // admin
    offset += U64_LEN * 2; // lp_fee_basis_points + protocol_fee_basis_points
    offset += U8_LEN; // disable_flags

    let protocol_fee_recipients = read_pubkey_array::<8>(data, offset)?;
    offset += PUBKEY_LEN * 8;
    offset += U64_LEN; // coin_creator_fee_basis_points
    offset += PUBKEY_LEN; // admin_set_coin_creator_authority
    offset += PUBKEY_LEN; // whitelist_pda

    let reserved_fee_recipient = read_pubkey(data, offset)?;
    offset += PUBKEY_LEN;
    offset += BOOL_LEN; // mayhem_mode_enabled

    let reserved_fee_recipients = read_pubkey_array::<7>(data, offset)?;
    offset += PUBKEY_LEN * 7;
    offset += BOOL_LEN; // is_cashback_enabled

    let buyback_fee_recipients = read_pubkey_array::<8>(data, offset)?;

    Some(GlobalConfig {
        protocol_fee_recipients,
        reserved_fee_recipient,
        reserved_fee_recipients,
        buyback_fee_recipients,
    })
}

async fn refresh_global_config_once(rpc: &SolanaRpcClient) -> Option<GlobalConfig> {
    let account = match tokio::time::timeout(
        PUMPSWAP_GLOBAL_CONFIG_RPC_TIMEOUT,
        rpc.get_account(&accounts::GLOBAL_ACCOUNT),
    )
    .await
    {
        Ok(Ok(account)) => account,
        Ok(Err(e)) => {
            warn!(target: "pumpswap_global_config", "PumpSwap GlobalConfig 读取失败: {}", e);
            return None;
        }
        Err(_) => {
            warn!(
                target: "pumpswap_global_config",
                timeout_ms = PUMPSWAP_GLOBAL_CONFIG_RPC_TIMEOUT.as_millis(),
                "PumpSwap GlobalConfig 读取超时"
            );
            return None;
        }
    };

    let Some(config) = decode_global_config(&account.data) else {
        warn!(
            target: "pumpswap_global_config",
            data_len = account.data.len(),
            "PumpSwap GlobalConfig 解析失败"
        );
        return None;
    };

    *GLOBAL_CONFIG_CACHE.write() =
        Some(CachedGlobalConfig { fetched_at: Instant::now(), config: config.clone() });
    Some(config)
}

pub async fn warm_pumpswap_global_config(rpc: Option<&Arc<SolanaRpcClient>>) {
    let Some(rpc) = rpc else {
        return;
    };
    let stale = GLOBAL_CONFIG_CACHE
        .read()
        .as_ref()
        .map(|c| c.fetched_at.elapsed() > PUMPSWAP_GLOBAL_CONFIG_TTL)
        .unwrap_or(true);
    if stale && !GLOBAL_CONFIG_REFRESH_IN_FLIGHT.swap(true, Ordering::AcqRel) {
        let rpc = Arc::clone(rpc);
        tokio::spawn(async move {
            let _ = refresh_global_config_once(rpc.as_ref()).await;
            GLOBAL_CONFIG_REFRESH_IN_FLIGHT.store(false, Ordering::Release);
        });
    }
}

fn cached_global_config() -> Option<GlobalConfig> {
    let guard = GLOBAL_CONFIG_CACHE.read();
    let cached = guard.as_ref()?;
    (cached.fetched_at.elapsed() <= PUMPSWAP_GLOBAL_CONFIG_TTL).then(|| cached.config.clone())
}

fn choose_nonzero(keys: &[Pubkey]) -> Option<Pubkey> {
    let mut valid = [Pubkey::default(); 8];
    let mut len = 0;
    for key in keys.iter().copied() {
        if key == Pubkey::default() || len == valid.len() {
            continue;
        }
        valid[len] = key;
        len += 1;
    }
    valid[..len].choose(&mut rand::rng()).copied()
}

/// Returns a random Mayhem fee recipient and its AccountMeta (pump-public-docs: use any one randomly).
#[inline]
pub fn get_mayhem_fee_recipient_random() -> (Pubkey, AccountMeta) {
    let recipient = cached_global_config()
        .and_then(|config| {
            let mut pool = [Pubkey::default(); 8];
            pool[0] = config.reserved_fee_recipient;
            pool[1..].copy_from_slice(&config.reserved_fee_recipients);
            choose_nonzero(&pool)
        })
        .unwrap_or_else(|| {
            *accounts::MAYHEM_FEE_RECIPIENTS
                .choose(&mut rand::rng())
                .unwrap_or(&accounts::MAYHEM_FEE_RECIPIENTS[0])
        });
    let meta = AccountMeta { pubkey: recipient, is_signer: false, is_writable: false };
    (recipient, meta)
}

#[inline]
pub fn get_protocol_fee_recipient_random() -> Pubkey {
    cached_global_config()
        .and_then(|config| choose_nonzero(&config.protocol_fee_recipients))
        .unwrap_or(accounts::FEE_RECIPIENT)
}

/// Random entry from [`accounts::PROTOCOL_EXTRA_FEE_RECIPIENTS`] (readonly; paired with [`fee_recipient_ata`] as last account).
#[inline]
pub fn get_protocol_extra_fee_recipient_random() -> Pubkey {
    cached_global_config()
        .and_then(|config| choose_nonzero(&config.buyback_fee_recipients))
        .unwrap_or_else(|| {
            *accounts::PROTOCOL_EXTRA_FEE_RECIPIENTS
                .choose(&mut rand::rng())
                .unwrap_or(&accounts::PROTOCOL_EXTRA_FEE_RECIPIENTS[0])
        })
}

/// Pool v2 PDA (seeds: ["pool-v2", base_mint]). Required at end of buy/sell/buy_exact_quote_in accounts.
#[inline]
pub fn get_pool_v2_pda(base_mint: &Pubkey) -> Option<Pubkey> {
    let (pda, _) = Pubkey::find_program_address(
        &[seeds::POOL_V2_SEED, base_mint.as_ref()],
        &accounts::AMM_PROGRAM,
    );
    Some(pda)
}

/// Pump 程序上的 pool-authority PDA（canonical pool 的 creator），与 @pump-fun/pump-swap-sdk 一致。
#[inline]
pub fn get_pump_pool_authority_pda(mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[seeds::POOL_AUTHORITY_SEED, mint.as_ref()],
        &accounts::PUMP_PROGRAM_ID,
    )
    .0
}

/// Canonical Pump 池 PDA：index=0，creator=pumpPoolAuthorityPda(mint)，base_mint=mint，quote_mint=WSOL。
/// 与 @pump-fun/pump-swap-sdk 的 canonicalPumpPoolPda(mint) 一致，用于从 bonding curve 迁移后的标准池查找。
#[inline]
pub fn get_canonical_pool_pda(mint: &Pubkey) -> Pubkey {
    const CANONICAL_POOL_INDEX: u16 = 0;
    let authority = get_pump_pool_authority_pda(mint);
    let (pda, _) = Pubkey::find_program_address(
        &[
            seeds::POOL_SEED,
            &CANONICAL_POOL_INDEX.to_le_bytes(),
            authority.as_ref(),
            mint.as_ref(),
            WSOL_TOKEN_ACCOUNT.as_ref(),
        ],
        &accounts::AMM_PROGRAM,
    );
    pda
}

// Find a pool for a specific mint
pub async fn find_pool(rpc: &SolanaRpcClient, mint: &Pubkey) -> Result<Pubkey, anyhow::Error> {
    let (pool_address, _) = find_by_mint(rpc, mint).await?;
    Ok(pool_address)
}

pub(crate) fn coin_creator_vault_authority(coin_creator: Pubkey) -> Pubkey {
    let (pump_pool_authority, _) = Pubkey::find_program_address(
        &[b"creator_vault", &coin_creator.to_bytes()],
        &accounts::AMM_PROGRAM,
    );
    pump_pool_authority
}

pub(crate) fn coin_creator_vault_ata(coin_creator: Pubkey, quote_mint: Pubkey) -> Pubkey {
    let creator_vault_authority = coin_creator_vault_authority(coin_creator);
    let associated_token_creator_vault_authority = get_associated_token_address_with_program_id(
        &creator_vault_authority,
        &quote_mint,
        &TOKEN_PROGRAM,
    );
    associated_token_creator_vault_authority
}

pub(crate) fn fee_recipient_ata(fee_recipient: Pubkey, quote_mint: Pubkey) -> Pubkey {
    let associated_token_fee_recipient =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &fee_recipient,
            &quote_mint,
            &TOKEN_PROGRAM,
        );
    associated_token_fee_recipient
}

pub fn get_user_volume_accumulator_pda(user: &Pubkey) -> Option<Pubkey> {
    crate::common::fast_fn::get_cached_pda(
        crate::common::fast_fn::PdaCacheKey::PumpSwapUserVolume(*user),
        || {
            let seeds: &[&[u8]; 2] = &[&seeds::USER_VOLUME_ACCUMULATOR_SEED, user.as_ref()];
            let program_id: &Pubkey = &accounts::AMM_PROGRAM;
            let pda: Option<(Pubkey, u8)> = Pubkey::try_find_program_address(seeds, program_id);
            pda.map(|pubkey| pubkey.0)
        },
    )
}

/// WSOL ATA of UserVolumeAccumulator for Pump AMM (buy cashback: remaining_accounts[0] 官方用 NATIVE_MINT).
pub fn get_user_volume_accumulator_wsol_ata(user: &Pubkey) -> Option<Pubkey> {
    let accumulator = get_user_volume_accumulator_pda(user)?;
    Some(crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
        &accumulator,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
        &crate::constants::TOKEN_PROGRAM,
    ))
}

/// Quote-mint ATA of UserVolumeAccumulator（sell cashback 时官方用 quoteMint，非固定 WSOL）.
pub fn get_user_volume_accumulator_quote_ata(
    user: &Pubkey,
    quote_mint: &Pubkey,
    quote_token_program: &Pubkey,
) -> Option<Pubkey> {
    let accumulator = get_user_volume_accumulator_pda(user)?;
    Some(crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
        &accumulator,
        quote_mint,
        quote_token_program,
    ))
}

pub fn get_global_volume_accumulator_pda() -> Option<Pubkey> {
    let seeds: &[&[u8]; 1] = &[&seeds::GLOBAL_VOLUME_ACCUMULATOR_SEED];
    let program_id: &Pubkey = &accounts::AMM_PROGRAM;
    let pda: Option<(Pubkey, u8)> = Pubkey::try_find_program_address(seeds, program_id);
    pda.map(|pubkey| pubkey.0)
}

pub async fn fetch_pool(
    rpc: &SolanaRpcClient,
    pool_address: &Pubkey,
) -> Result<Pool, anyhow::Error> {
    let account = rpc.get_account(pool_address).await?;
    if account.owner != accounts::AMM_PROGRAM {
        return Err(anyhow!("Account is not owned by PumpSwap program"));
    }
    let pool = pool_decode(&account.data[8..]).ok_or_else(|| anyhow!("Failed to decode pool"))?;
    Ok(pool)
}

/// Known pool account sizes: 252 (SPL Token) and 643 (Token2022)
const POOL_DATA_LEN_SPL: u64 = 8 + 244;
const POOL_DATA_LEN_T22: u64 = 643;

/// Run getProgramAccounts with a Memcmp filter, querying both pool sizes in parallel.
async fn get_program_accounts_both_sizes(
    rpc: &SolanaRpcClient,
    memcmp_offset: usize,
    mint: &Pubkey,
) -> Result<Vec<(Pubkey, solana_sdk::account::Account)>, anyhow::Error> {
    let make_config = |data_size: u64| solana_rpc_client_api::config::RpcProgramAccountsConfig {
        filters: Some(vec![
            solana_rpc_client_api::filter::RpcFilterType::DataSize(data_size),
            solana_rpc_client_api::filter::RpcFilterType::Memcmp(
                solana_client::rpc_filter::Memcmp::new_base58_encoded(memcmp_offset, mint.as_ref()),
            ),
        ]),
        account_config: solana_rpc_client_api::config::RpcAccountInfoConfig {
            encoding: Some(UiAccountEncoding::Base64),
            data_slice: None,
            commitment: None,
            min_context_slot: None,
        },
        with_context: None,
        sort_results: None,
    };
    let program_id = accounts::AMM_PROGRAM;
    #[allow(deprecated)]
    let (spl_result, t22_result) = tokio::join!(
        rpc.get_program_accounts_with_config(&program_id, make_config(POOL_DATA_LEN_SPL)),
        rpc.get_program_accounts_with_config(&program_id, make_config(POOL_DATA_LEN_T22)),
    );
    let mut all = spl_result.unwrap_or_default();
    all.extend(t22_result.unwrap_or_default());
    Ok(all)
}

fn decode_pool_accounts(
    accounts: Vec<(Pubkey, solana_sdk::account::Account)>,
) -> Vec<(Pubkey, Pool)> {
    accounts
        .into_iter()
        .filter_map(|(addr, acc)| {
            if acc.data.len() > 8 {
                pool_decode(&acc.data[8..]).map(|pool| (addr, pool))
            } else {
                None
            }
        })
        .collect()
}

pub async fn find_by_base_mint(
    rpc: &SolanaRpcClient,
    base_mint: &Pubkey,
) -> Result<(Pubkey, Pool), anyhow::Error> {
    // base_mint offset: 8(discriminator) + 1(bump) + 2(index) + 32(creator) = 43
    let accounts = get_program_accounts_both_sizes(rpc, 43, base_mint).await?;
    if accounts.is_empty() {
        return Err(anyhow!("No pool found for mint {}", base_mint));
    }
    let mut pools = decode_pool_accounts(accounts);
    if pools.is_empty() {
        return Err(anyhow!("No valid pool decoded for mint {}", base_mint));
    }
    pools.sort_by(|a, b| b.1.lp_supply.cmp(&a.1.lp_supply));
    Ok((pools[0].0, pools[0].1.clone()))
}

pub async fn find_by_quote_mint(
    rpc: &SolanaRpcClient,
    quote_mint: &Pubkey,
) -> Result<(Pubkey, Pool), anyhow::Error> {
    // quote_mint offset: 8 + 1 + 2 + 32 + 32 = 75
    let accounts = get_program_accounts_both_sizes(rpc, 75, quote_mint).await?;
    if accounts.is_empty() {
        return Err(anyhow!("No pool found for mint {}", quote_mint));
    }
    let mut pools = decode_pool_accounts(accounts);
    if pools.is_empty() {
        return Err(anyhow!("No valid pool decoded for quote_mint {}", quote_mint));
    }
    pools.sort_by(|a, b| b.1.lp_supply.cmp(&a.1.lp_supply));
    Ok((pools[0].0, pools[0].1.clone()))
}

/// 按 mint 查找 PumpSwap 池（本函数仅用于 PumpSwap，其他 DEX 勿用）。
///
/// 查找顺序（与 @pump-fun/pump-swap-sdk 一致）：
/// 1. Pool v2 PDA ["pool-v2", base_mint] — 一次 getAccount
/// 2. Canonical pool PDA ["pool", 0, pumpPoolAuthority(mint), mint, WSOL] — 迁移后的标准池
/// 3. getProgramAccounts 按 base_mint / quote_mint 过滤
pub async fn find_by_mint(
    rpc: &SolanaRpcClient,
    mint: &Pubkey,
) -> Result<(Pubkey, Pool), anyhow::Error> {
    let mut diag = Vec::<String>::new();

    // 1. PumpSwap v2 PDA（seeds: ["pool-v2", base_mint]）
    if let Some(pool_address) = get_pool_v2_pda(mint) {
        diag.push(format!("PDA(v2)={}", pool_address));
        match fetch_pool(rpc, &pool_address).await {
            Ok(pool) if pool.base_mint == *mint => return Ok((pool_address, pool)),
            Ok(_) => diag.push("PDA(v2) 账户存在但 base_mint 不匹配".into()),
            Err(e) => diag.push(format!("PDA(v2) get_account/decode 失败: {}", e)),
        }
    }

    // 2. Canonical pool PDA（与 pump-swap-sdk canonicalPumpPoolPda(mint) 一致）
    let canonical_address = get_canonical_pool_pda(mint);
    diag.push(format!("canonical={}", canonical_address));
    match fetch_pool(rpc, &canonical_address).await {
        Ok(pool) if pool.base_mint == *mint => return Ok((canonical_address, pool)),
        Ok(_) => diag.push("canonical 账户存在但 base_mint 不匹配".into()),
        Err(e) => diag.push(format!("canonical get_account/decode 失败: {}", e)),
    }

    // 3. Fallback: getProgramAccounts by base_mint / quote_mint (with 3s timeout to avoid blocking)
    match tokio::time::timeout(std::time::Duration::from_secs(3), find_by_base_mint(rpc, mint))
        .await
    {
        Ok(Ok((address, pool))) => return Ok((address, pool)),
        Ok(Err(e)) => diag.push(format!("getProgramAccounts(base_mint): {}", e)),
        Err(_) => diag.push("getProgramAccounts(base_mint): timed out (3s)".into()),
    }
    match tokio::time::timeout(std::time::Duration::from_secs(3), find_by_quote_mint(rpc, mint))
        .await
    {
        Ok(Ok((address, pool))) => return Ok((address, pool)),
        Ok(Err(e)) => diag.push(format!("getProgramAccounts(quote_mint): {}", e)),
        Err(_) => diag.push("getProgramAccounts(quote_mint): timed out (3s)".into()),
    }

    let diag_str = diag.join("; ");
    eprintln!("[find_by_mint] {} failed: {}", mint, diag_str);
    Err(anyhow!("No pool found for mint {}. diag: {}", mint, diag_str))
}

pub async fn get_token_balances(
    pool: &Pool,
    rpc: &SolanaRpcClient,
) -> Result<(u64, u64), anyhow::Error> {
    let base_balance = rpc.get_token_account_balance(&pool.pool_base_token_account).await?;
    let quote_balance = rpc.get_token_account_balance(&pool.pool_quote_token_account).await?;

    let base_amount = base_balance.amount.parse::<u64>().map_err(|e| anyhow!(e))?;
    let quote_amount = quote_balance.amount.parse::<u64>().map_err(|e| anyhow!(e))?;

    Ok((base_amount, quote_amount))
}

#[inline]
pub fn get_fee_config_pda() -> Option<Pubkey> {
    let seeds: &[&[u8]; 2] = &[seeds::FEE_CONFIG_SEED, accounts::AMM_PROGRAM.as_ref()];
    let program_id: &Pubkey = &accounts::FEE_PROGRAM;
    let pda: Option<(Pubkey, u8)> = Pubkey::try_find_program_address(seeds, program_id);
    pda.map(|pubkey| pubkey.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;

    #[test]
    fn pumpswap_user_volume_accumulator_pda_deterministic() {
        let user = Pubkey::new_unique();
        let a = get_user_volume_accumulator_pda(&user).unwrap();
        let b = get_user_volume_accumulator_pda(&user).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn pumpswap_global_volume_accumulator_matches_constant() {
        let pda = get_global_volume_accumulator_pda().unwrap();
        assert_eq!(pda, accounts::GLOBAL_VOLUME_ACCUMULATOR);
    }

    #[test]
    fn pumpswap_pool_v2_pda_deterministic() {
        let base_mint = Pubkey::new_unique();
        let a = get_pool_v2_pda(&base_mint).unwrap();
        let b = get_pool_v2_pda(&base_mint).unwrap();
        assert_eq!(a, b);
    }
}
