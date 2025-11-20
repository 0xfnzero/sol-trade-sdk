use crate::{
    common::SolanaRpcClient,
    trading::core::params::RaydiumCpmmParams,
};
use anyhow::anyhow;
use solana_sdk::pubkey::Pubkey;
use solana_streamer::streaming::event_parser::protocols::raydium_cpmm::types::{pool_state_decode, PoolState};

/// Constants used as seeds for deriving PDAs (Program Derived Addresses)
pub mod seeds {
    pub const POOL_SEED: &[u8] = b"pool";
    pub const POOL_VAULT_SEED: &[u8] = b"pool_vault";
    pub const OBSERVATION_STATE_SEED: &[u8] = b"observation";
}

/// Constants related to program accounts and authorities
pub mod accounts {
    use solana_sdk::{pubkey, pubkey::Pubkey};
    pub const AUTHORITY: Pubkey = pubkey!("GpMZbSM2GgvTKHJirzeGfMFoaZ8UR2X7F4v8vHTvxFbL");
    pub const RAYDIUM_CPMM: Pubkey = pubkey!("CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C");
    pub const FEE_RATE_DENOMINATOR_VALUE: u128 = 1_000_000;
    pub const TRADE_FEE_RATE: u64 = 2500;
    pub const CREATOR_FEE_RATE: u64 = 0;
    pub const PROTOCOL_FEE_RATE: u64 = 120000;
    pub const FUND_FEE_RATE: u64 = 40000;
    // META
    pub const AUTHORITY_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: AUTHORITY,
            is_signer: false,
            is_writable: false,
        };
}

pub const SWAP_BASE_IN_DISCRIMINATOR: &[u8] = &[143, 190, 90, 218, 196, 30, 51, 222];
pub const SWAP_BASE_OUT_DISCRIMINATOR: &[u8] = &[55, 217, 98, 86, 163, 74, 180, 173];

pub async fn fetch_pool_state(
    rpc: &SolanaRpcClient,
    pool_address: &Pubkey,
) -> Result<PoolState, anyhow::Error> {
    let account = rpc.get_account(pool_address).await?;
    if account.owner != accounts::RAYDIUM_CPMM {
        return Err(anyhow!("Account is not owned by Raydium Cpmm program"));
    }
    let pool_state = pool_state_decode(&account.data[8..])
        .ok_or_else(|| anyhow!("Failed to decode pool state"))?;
    Ok(pool_state)
}

pub fn get_pool_pda(amm_config: &Pubkey, mint1: &Pubkey, mint2: &Pubkey) -> Option<Pubkey> {
    let seeds: &[&[u8]; 4] =
        &[seeds::POOL_SEED, amm_config.as_ref(), mint1.as_ref(), mint2.as_ref()];
    let program_id: &Pubkey = &accounts::RAYDIUM_CPMM;
    let pda: Option<(Pubkey, u8)> = Pubkey::try_find_program_address(seeds, program_id);
    pda.map(|pubkey| pubkey.0)
}

pub fn get_vault_pda(pool_state: &Pubkey, mint: &Pubkey) -> Option<Pubkey> {
    let seeds: &[&[u8]; 3] = &[seeds::POOL_VAULT_SEED, pool_state.as_ref(), mint.as_ref()];
    let program_id: &Pubkey = &accounts::RAYDIUM_CPMM;
    let pda: Option<(Pubkey, u8)> = Pubkey::try_find_program_address(seeds, program_id);
    pda.map(|pubkey| pubkey.0)
}

pub fn get_observation_state_pda(pool_state: &Pubkey) -> Option<Pubkey> {
    let seeds: &[&[u8]; 2] = &[seeds::OBSERVATION_STATE_SEED, pool_state.as_ref()];
    let program_id: &Pubkey = &accounts::RAYDIUM_CPMM;
    let pda: Option<(Pubkey, u8)> = Pubkey::try_find_program_address(seeds, program_id);
    pda.map(|pubkey| pubkey.0)
}

/// Get the balances of two tokens in the pool
///
/// # Returns
/// Returns token0_balance, token1_balance
pub async fn get_pool_token_balances(
    rpc: &SolanaRpcClient,
    pool_state: &Pubkey,
    token0_mint: &Pubkey,
    token1_mint: &Pubkey,
) -> Result<(u64, u64), anyhow::Error> {
    let token0_vault = get_vault_pda(pool_state, token0_mint).unwrap();
    let token0_balance = rpc.get_token_account_balance(&token0_vault).await?;
    let token1_vault = get_vault_pda(pool_state, token1_mint).unwrap();
    let token1_balance = rpc.get_token_account_balance(&token1_vault).await?;

    // Parse balance string to u64
    let token0_amount = token0_balance
        .amount
        .parse::<u64>()
        .map_err(|e| anyhow!("Failed to parse token0 balance: {}", e))?;

    let token1_amount = token1_balance
        .amount
        .parse::<u64>()
        .map_err(|e| anyhow!("Failed to parse token1 balance: {}", e))?;

    Ok((token0_amount, token1_amount))
}

/// Calculate token price (token1/token0)
///
/// # Returns
/// Returns the price of token1 relative to token0
pub async fn calculate_price(
    token0_amount: u64,
    token1_amount: u64,
    mint0_decimals: u8,
    mint1_decimals: u8,
) -> Result<f64, anyhow::Error> {
    if token0_amount == 0 {
        return Err(anyhow!("Token0 balance is zero, cannot calculate price"));
    }
    // Consider decimal precision
    let token0_adjusted = token0_amount as f64 / 10_f64.powi(mint0_decimals as i32);
    let token1_adjusted = token1_amount as f64 / 10_f64.powi(mint1_decimals as i32);
    let price = token1_adjusted / token0_adjusted;
    Ok(price)
}

/// Helper function to get token vault account address
///
/// # Parameters
/// - `pool_state`: Pool state account address
/// - `token_mint`: Token mint address
/// - `protocol_params`: Protocol parameters
///
/// # Returns
/// Returns the corresponding token vault account address
pub fn get_vault_account(
    pool_state: &Pubkey,
    token_mint: &Pubkey,
    protocol_params: &RaydiumCpmmParams,
) -> Pubkey {
    if protocol_params.token_mint_0 == *token_mint && protocol_params.token_vault_0 != Pubkey::default() {
        protocol_params.token_vault_0
    } else if protocol_params.token_mint_1 == *token_mint && protocol_params.token_vault_1 != Pubkey::default() {
        protocol_params.token_vault_1
    } else {
        get_vault_pda(pool_state, token_mint).unwrap()
    }
}