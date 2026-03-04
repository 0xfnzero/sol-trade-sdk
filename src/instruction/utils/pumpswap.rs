use crate::{
    common::{
        spl_associated_token_account::get_associated_token_address_with_program_id, SolanaRpcClient,
    },
    constants::{TOKEN_PROGRAM, WSOL_TOKEN_ACCOUNT},
    instruction::utils::pumpswap_types::{pool_decode, Pool},
};
use anyhow::anyhow;
use solana_account_decoder::UiAccountEncoding;
use solana_sdk::pubkey::Pubkey;

/// PumpSwap 池账户总长度（见 pump-public-docs Breaking Change）：8 字节 discriminator + 244 字节 Pool。
/// 官方文档：pool structure needs to be 244 bytes (was 243)，含 is_mayhem_mode。DataSize 必须与此一致，否则 getProgramAccounts 会返回 0。
const POOL_ACCOUNT_DATA_LEN: u64 = 8 + 244;

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

    /// Mayhem fee recipient (for mayhem mode coins)
    pub const MAYHEM_FEE_RECIPIENT: Pubkey =
        pubkey!("GesfTA3X2arioaHp8bbKdjG9vJtskViWACZoYvxp4twS");

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
            let program_id: &Pubkey = &&accounts::AMM_PROGRAM;
            let pda: Option<(Pubkey, u8)> = Pubkey::try_find_program_address(seeds, program_id);
            pda.map(|pubkey| pubkey.0)
        },
    )
}

/// WSOL ATA of UserVolumeAccumulator for Pump AMM (used for cashback remaining accounts).
pub fn get_user_volume_accumulator_wsol_ata(user: &Pubkey) -> Option<Pubkey> {
    let accumulator = get_user_volume_accumulator_pda(user)?;
    Some(crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
        &accumulator,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
        &crate::constants::TOKEN_PROGRAM,
    ))
}

pub fn get_global_volume_accumulator_pda() -> Option<Pubkey> {
    let seeds: &[&[u8]; 1] = &[&seeds::GLOBAL_VOLUME_ACCUMULATOR_SEED];
    let program_id: &Pubkey = &&accounts::AMM_PROGRAM;
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

pub async fn find_by_base_mint(
    rpc: &SolanaRpcClient,
    base_mint: &Pubkey,
) -> Result<(Pubkey, Pool), anyhow::Error> {
    // Use getProgramAccounts to find pools for the given mint.
    // base_mint 在账户布局中的偏移：8(discriminator) + 1(bump) + 2(index) + 32(creator) = 43
    let filters = vec![
        solana_rpc_client_api::filter::RpcFilterType::DataSize(POOL_ACCOUNT_DATA_LEN),
        solana_rpc_client_api::filter::RpcFilterType::Memcmp(
            solana_client::rpc_filter::Memcmp::new_base58_encoded(43, base_mint.as_ref()),
        ),
    ];
    let config = solana_rpc_client_api::config::RpcProgramAccountsConfig {
        filters: Some(filters),
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
    let accounts = rpc.get_program_accounts_with_config(&program_id, config).await?;
    if accounts.is_empty() {
        return Err(anyhow!("No pool found for mint {}", base_mint));
    }
    let accounts_count = accounts.len();  // 🔧 保存长度，因为 into_iter() 会消耗 accounts
    let mut pools: Vec<_> = accounts
        .into_iter()
        .filter_map(|(addr, acc)| {
            // 🔧 修复：跳过8字节的discriminator
            if acc.data.len() > 8 {
                pool_decode(&acc.data[8..]).map(|pool| (addr, pool))
            } else {
                None
            }
        })
        .collect();

    // 🔧 修复：检查过滤后的 pools 是否为空（accounts 可能不为空但解码全部失败）
    if pools.is_empty() {
        return Err(anyhow!("No valid pool decoded for mint {} (found {} accounts but all decode failed)", base_mint, accounts_count));
    }

    pools.sort_by(|a, b| b.1.lp_supply.cmp(&a.1.lp_supply));
    let (address, pool) = pools[0].clone();
    Ok((address, pool))
}

pub async fn find_by_quote_mint(
    rpc: &SolanaRpcClient,
    quote_mint: &Pubkey,
) -> Result<(Pubkey, Pool), anyhow::Error> {
    // Use getProgramAccounts to find pools for the given mint.
    // quote_mint 在账户布局中的偏移：8 + 1 + 2 + 32 + 32 = 75
    let filters = vec![
        solana_rpc_client_api::filter::RpcFilterType::DataSize(POOL_ACCOUNT_DATA_LEN),
        solana_rpc_client_api::filter::RpcFilterType::Memcmp(
            solana_client::rpc_filter::Memcmp::new_base58_encoded(75, quote_mint.as_ref()),
        ),
    ];
    let config = solana_rpc_client_api::config::RpcProgramAccountsConfig {
        filters: Some(filters),
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
    let accounts = rpc.get_program_accounts_with_config(&program_id, config).await?;
    if accounts.is_empty() {
        return Err(anyhow!("No pool found for mint {}", quote_mint));
    }
    let accounts_count = accounts.len();  // 🔧 保存长度，因为 into_iter() 会消耗 accounts
    let mut pools: Vec<_> = accounts
        .into_iter()
        .filter_map(|(addr, acc)| {
            // 🔧 修复：跳过8字节的discriminator
            if acc.data.len() > 8 {
                pool_decode(&acc.data[8..]).map(|pool| (addr, pool))
            } else {
                None
            }
        })
        .collect();

    // 🔧 修复：检查过滤后的 pools 是否为空（accounts 可能不为空但解码全部失败）
    if pools.is_empty() {
        return Err(anyhow!("No valid pool decoded for quote_mint {} (found {} accounts but all decode failed)", quote_mint, accounts_count));
    }

    pools.sort_by(|a, b| b.1.lp_supply.cmp(&a.1.lp_supply));
    let (address, pool) = pools[0].clone();
    Ok((address, pool))
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

    // 3. 回退：getProgramAccounts 按 base_mint / quote_mint
    match find_by_base_mint(rpc, mint).await {
        Ok((address, pool)) => return Ok((address, pool)),
        Err(e) => diag.push(format!("getProgramAccounts(base_mint): {}", e)),
    }
    match find_by_quote_mint(rpc, mint).await {
        Ok((address, pool)) => return Ok((address, pool)),
        Err(e) => diag.push(format!("getProgramAccounts(quote_mint): {}", e)),
    }

    Err(anyhow!(
        "No pool found for mint {}. 诊断: {}。若使用自建 RPC 请确认已开启 getProgramAccounts 或换用公共 RPC 重试；若代币未在 PumpSwap 建池请先在 pump.fun/DEX 上确认",
        mint,
        diag.join("; ")
    ))
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
