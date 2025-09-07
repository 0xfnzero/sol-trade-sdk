use crate::{
    common::SolanaRpcClient,
    constants::{self, TOKEN_PROGRAM},
};
use anyhow::anyhow;
use solana_account_decoder::UiAccountEncoding;
use solana_sdk::pubkey::Pubkey;
use solana_streamer_sdk::streaming::event_parser::protocols::pumpswap::types::{pool_decode, Pool};

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
}

/// Constants related to program accounts and authorities
pub mod accounts {
    use solana_sdk::{pubkey, pubkey::Pubkey};

    use crate::instruction::utils::pumpswap::{
        get_fee_config_pda, get_global_volume_accumulator_pda,
    };

    /// Public key for the fee recipient
    pub const FEE_RECIPIENT: Pubkey = pubkey!("62qc2CNXwrYqQScmEdiZFFAnJR262PxWEuNQtxfafNgV");

    /// Public key for the global PDA
    pub const GLOBAL_ACCOUNT: Pubkey = pubkey!("ADyA8hdefvWN2dbGGWFotbzWxrAvLW83WG6QCVXvJKqw");

    /// Authority for program events
    pub const EVENT_AUTHORITY: Pubkey = pubkey!("GS4CU59F31iL7aR2Q8zVS8DRrcRnXX1yjQ66TqNVQnaR");

    /// Associated Token Program ID
    pub const ASSOCIATED_TOKEN_PROGRAM: Pubkey =
        pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

    // PumpSwap 协议费用接收者
    pub const PROTOCOL_FEE_RECIPIENT: Pubkey =
        pubkey!("62qc2CNXwrYqQScmEdiZFFAnJR262PxWEuNQtxfafNgV");

    /// Rent Sysvar ID
    pub const RENT: Pubkey = pubkey!("SysvarRent111111111111111111111111111111111");

    pub const AMM_PROGRAM: Pubkey = pubkey!("pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA");

    pub const LP_FEE_BASIS_POINTS: u64 = 20;
    pub const PROTOCOL_FEE_BASIS_POINTS: u64 = 5;
    pub const COIN_CREATOR_FEE_BASIS_POINTS: u64 = 5;

    pub const FEE_PROGRAM: Pubkey = pubkey!("pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ");

    // META

    pub const GLOBAL_ACCOUNT_META: once_cell::sync::Lazy<solana_sdk::instruction::AccountMeta> =
        once_cell::sync::Lazy::new(|| {
            solana_sdk::instruction::AccountMeta::new_readonly(GLOBAL_ACCOUNT, false)
        });

    pub const FEE_RECIPIENT_META: once_cell::sync::Lazy<solana_sdk::instruction::AccountMeta> =
        once_cell::sync::Lazy::new(|| {
            solana_sdk::instruction::AccountMeta::new_readonly(FEE_RECIPIENT, false)
        });

    pub const ASSOCIATED_TOKEN_PROGRAM_META: once_cell::sync::Lazy<
        solana_sdk::instruction::AccountMeta,
    > = once_cell::sync::Lazy::new(|| {
        solana_sdk::instruction::AccountMeta::new_readonly(ASSOCIATED_TOKEN_PROGRAM, false)
    });

    pub const EVENT_AUTHORITY_META: once_cell::sync::Lazy<solana_sdk::instruction::AccountMeta> =
        once_cell::sync::Lazy::new(|| {
            solana_sdk::instruction::AccountMeta::new_readonly(EVENT_AUTHORITY, false)
        });

    pub const AMM_PROGRAM_META: once_cell::sync::Lazy<solana_sdk::instruction::AccountMeta> =
        once_cell::sync::Lazy::new(|| {
            solana_sdk::instruction::AccountMeta::new_readonly(AMM_PROGRAM, false)
        });

    pub const GLOBAL_VOLUME_ACCUMULATOR_META: once_cell::sync::Lazy<
        solana_sdk::instruction::AccountMeta,
    > = once_cell::sync::Lazy::new(|| {
        solana_sdk::instruction::AccountMeta::new_readonly(
            get_global_volume_accumulator_pda().unwrap(),
            false,
        )
    });

    pub const FEE_CONFIG_META: once_cell::sync::Lazy<solana_sdk::instruction::AccountMeta> =
        once_cell::sync::Lazy::new(|| {
            solana_sdk::instruction::AccountMeta::new_readonly(get_fee_config_pda().unwrap(), false)
        });

    pub const FEE_PROGRAM_META: once_cell::sync::Lazy<solana_sdk::instruction::AccountMeta> =
        once_cell::sync::Lazy::new(|| {
            solana_sdk::instruction::AccountMeta::new_readonly(FEE_PROGRAM, false)
        });
}

pub const BUY_DISCRIMINATOR: [u8; 8] = [102, 6, 61, 18, 1, 218, 235, 234];
pub const SELL_DISCRIMINATOR: [u8; 8] = [51, 230, 133, 164, 1, 127, 131, 173];

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
    let associated_token_creator_vault_authority =
        spl_associated_token_account::get_associated_token_address_with_program_id(
            &creator_vault_authority,
            &quote_mint,
            &TOKEN_PROGRAM,
        );
    associated_token_creator_vault_authority
}

pub(crate) fn fee_recipient_ata(fee_recipient: Pubkey, quote_mint: Pubkey) -> Pubkey {
    let associated_token_fee_recipient =
        spl_associated_token_account::get_associated_token_address_with_program_id(
            &fee_recipient,
            &quote_mint,
            &TOKEN_PROGRAM,
        );
    associated_token_fee_recipient
}

pub fn get_user_volume_accumulator_pda(user: &Pubkey) -> Option<Pubkey> {
    let seeds: &[&[u8]; 2] = &[&seeds::USER_VOLUME_ACCUMULATOR_SEED, user.as_ref()];
    let program_id: &Pubkey = &&accounts::AMM_PROGRAM;
    let pda: Option<(Pubkey, u8)> = Pubkey::try_find_program_address(seeds, program_id);
    pda.map(|pubkey| pubkey.0)
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
    // 使用getProgramAccounts查找给定mint的池子
    let filters = vec![
        // solana_rpc_client_api::filter::RpcFilterType::DataSize(211), // Pool账户的大小
        solana_rpc_client_api::filter::RpcFilterType::Memcmp(
            solana_client::rpc_filter::Memcmp::new_base58_encoded(43, &base_mint.to_bytes()),
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
    let accounts = rpc.get_program_accounts_with_config(&program_id, config).await?;
    if accounts.is_empty() {
        return Err(anyhow!("No pool found for mint {}", base_mint));
    }
    let mut pools: Vec<_> = accounts
        .into_iter()
        .filter_map(|(addr, acc)| pool_decode(&acc.data).map(|pool| (addr, pool)))
        .collect();
    pools.sort_by(|a, b| b.1.lp_supply.cmp(&a.1.lp_supply));
    let (address, pool) = pools[0].clone();
    Ok((address, pool))
}

pub async fn find_by_quote_mint(
    rpc: &SolanaRpcClient,
    quote_mint: &Pubkey,
) -> Result<(Pubkey, Pool), anyhow::Error> {
    // 使用getProgramAccounts查找给定mint的池子
    let filters = vec![
        // solana_rpc_client_api::filter::RpcFilterType::DataSize(211), // Pool账户的大小
        solana_rpc_client_api::filter::RpcFilterType::Memcmp(
            solana_client::rpc_filter::Memcmp::new_base58_encoded(75, &quote_mint.to_bytes()),
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
    let accounts = rpc.get_program_accounts_with_config(&program_id, config).await?;
    if accounts.is_empty() {
        return Err(anyhow!("No pool found for mint {}", quote_mint));
    }
    let mut pools: Vec<_> = accounts
        .into_iter()
        .filter_map(|(addr, acc)| pool_decode(&acc.data).map(|pool| (addr, pool)))
        .collect();
    pools.sort_by(|a, b| b.1.lp_supply.cmp(&a.1.lp_supply));
    let (address, pool) = pools[0].clone();
    Ok((address, pool))
}

pub async fn find_by_mint(
    rpc: &SolanaRpcClient,
    mint: &Pubkey,
) -> Result<(Pubkey, Pool), anyhow::Error> {
    if let Ok((address, pool)) = find_by_base_mint(rpc, mint).await {
        return Ok((address, pool));
    }
    if let Ok((address, pool)) = find_by_quote_mint(rpc, mint).await {
        return Ok((address, pool));
    }
    Err(anyhow!("No pool found for mint {}", mint))
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
