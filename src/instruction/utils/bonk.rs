use crate::{
    common::SolanaRpcClient,
    instruction::utils::bonk_types::{
        pool_state_decode, PoolState, POOL_STATE_DISCRIMINATOR, POOL_STATE_SIZE,
    },
    utils::calc::common::clamp_slippage_basis_points_u128,
};
use anyhow::anyhow;
use solana_sdk::pubkey::Pubkey;

/// Constants used as seeds for deriving PDAs (Program Derived Addresses)
pub mod seeds {
    pub const POOL_SEED: &[u8] = b"pool";
    pub const POOL_VAULT_SEED: &[u8] = b"pool_vault";
}

/// Constants related to program accounts and authorities
pub mod accounts {
    use solana_sdk::{pubkey, pubkey::Pubkey};

    pub const AUTHORITY: Pubkey = pubkey!("WLHv2UAZm6z4KyaaELi5pjdbJh6RESMva1Rnn8pJVVh");
    pub const GLOBAL_CONFIG: Pubkey = pubkey!("6s1xP3hpbAfFoNtUNF8mfHsjr2Bd97JxFJRWLbL6aHuX");
    pub const USD1_GLOBAL_CONFIG: Pubkey = pubkey!("EPiZbnrThjyLnoQ6QQzkxeFqyL5uyg9RzNHHAudUPxBz");
    pub const EVENT_AUTHORITY: Pubkey = pubkey!("2DPAtwB8L12vrMRExbLuyGnC7n2J5LNoZQSejeQGpwkr");
    pub const BONK: Pubkey = pubkey!("LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj");
    pub const STONKFUN_STANDARD_PLATFORM_CONFIG: Pubkey =
        pubkey!("4E876qZTE9FJMrBzgVtBrSrzz2TLivB5Y5QXPjB4gZL7");
    pub const STONKFUN_REWARD_PLATFORM_CONFIG: Pubkey =
        pubkey!("6BwHHDg3u1854jC8PDLXvR4spTcLNaoBxLJNGC4nTESt");

    pub const PLATFORM_FEE_RATE: u128 = 100; // 1%
    pub const PROTOCOL_FEE_RATE: u128 = 25; // 0.25%
    pub const SHARE_FEE_RATE: u128 = 0; // 0%

    // META
    pub const AUTHORITY_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: AUTHORITY,
            is_signer: false,
            is_writable: false,
        };
    pub const GLOBAL_CONFIG_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: GLOBAL_CONFIG,
            is_signer: false,
            is_writable: false,
        };

    pub const USD1_GLOBAL_CONFIG_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: USD1_GLOBAL_CONFIG,
            is_signer: false,
            is_writable: false,
        };

    pub const EVENT_AUTHORITY_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: EVENT_AUTHORITY,
            is_signer: false,
            is_writable: false,
        };
    pub const BONK_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta { pubkey: BONK, is_signer: false, is_writable: false };
}

pub const BUY_EXECT_IN_DISCRIMINATOR: [u8; 8] = [250, 234, 13, 123, 213, 156, 19, 236];
pub const BUY_EXECT_OUT_DISCRIMINATOR: [u8; 8] = [24, 211, 116, 40, 105, 3, 153, 56];
pub const SELL_EXECT_IN_DISCRIMINATOR: [u8; 8] = [149, 39, 222, 155, 211, 124, 152, 26];
pub const SELL_EXECT_OUT_DISCRIMINATOR: [u8; 8] = [95, 200, 71, 34, 8, 9, 11, 166];
const GLOBAL_CONFIG_DISCRIMINATOR: [u8; 8] = [149, 8, 156, 202, 160, 252, 176, 217];
const PLATFORM_CONFIG_DISCRIMINATOR: [u8; 8] = [160, 78, 128, 0, 248, 83, 230, 160];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LaunchLabFeeConfig {
    pub curve_type: u8,
    pub trade_fee_rate: u64,
    pub platform_fee_rate: u64,
    pub creator_fee_rate: u64,
}

fn read_config_u64(data: &[u8], offset: usize, label: &str) -> Result<u64, anyhow::Error> {
    let bytes = data
        .get(offset..offset + 8)
        .ok_or_else(|| anyhow!("LaunchLab {} account is too short", label))?;
    let mut value = [0u8; 8];
    value.copy_from_slice(bytes);
    Ok(u64::from_le_bytes(value))
}

pub async fn fetch_fee_config(
    rpc: &SolanaRpcClient,
    global_config: &Pubkey,
    platform_config: &Pubkey,
) -> Result<LaunchLabFeeConfig, anyhow::Error> {
    let global = rpc.get_account(global_config).await?;
    let platform = rpc.get_account(platform_config).await?;
    if global.owner != accounts::BONK || platform.owner != accounts::BONK {
        return Err(anyhow!("LaunchLab config account has an unexpected owner"));
    }
    decode_fee_config(&global.data, &platform.data)
}

/// Decodes subscription/cache bytes without network access. Owners must be checked by the caller.
pub fn decode_fee_config(
    global: &[u8],
    platform: &[u8],
) -> Result<LaunchLabFeeConfig, anyhow::Error> {
    if global.get(..8) != Some(GLOBAL_CONFIG_DISCRIMINATOR.as_slice()) {
        return Err(anyhow!("Account discriminator is not LaunchLab GlobalConfig"));
    }
    if platform.get(..8) != Some(PLATFORM_CONFIG_DISCRIMINATOR.as_slice()) {
        return Err(anyhow!("Account discriminator is not LaunchLab PlatformConfig"));
    }

    Ok(LaunchLabFeeConfig {
        curve_type: *global
            .get(16)
            .ok_or_else(|| anyhow!("LaunchLab GlobalConfig account is too short"))?,
        trade_fee_rate: read_config_u64(global, 27, "GlobalConfig")?,
        platform_fee_rate: read_config_u64(platform, 104, "PlatformConfig")?,
        creator_fee_rate: read_config_u64(platform, 720, "PlatformConfig")?,
    })
}

pub async fn fetch_pool_state(
    rpc: &SolanaRpcClient,
    pool_address: &Pubkey,
) -> Result<PoolState, anyhow::Error> {
    let account = rpc.get_account(pool_address).await?;
    if account.owner != accounts::BONK {
        return Err(anyhow!("Account is not owned by the LaunchLab program"));
    }
    let expected_len = 8 + POOL_STATE_SIZE;
    if account.data.len() < expected_len {
        return Err(anyhow!(
            "LaunchLab pool account is too short: expected at least {expected_len} bytes, got {}",
            account.data.len()
        ));
    }
    if account.data[..8] != POOL_STATE_DISCRIMINATOR {
        return Err(anyhow!("Account discriminator is not LaunchLab PoolState"));
    }
    let pool_state = pool_state_decode(&account.data[8..])
        .ok_or_else(|| anyhow!("Failed to decode pool state"))?;
    Ok(pool_state)
}

pub fn get_amount_in_net(
    amount_in: u64,
    protocol_fee_rate: u128,
    platform_fee_rate: u128,
    share_fee_rate: u128,
) -> u64 {
    let amount_in_u128 = amount_in as u128;
    let protocol_fee = (amount_in_u128 * protocol_fee_rate / 10000) as u128;
    let platform_fee = (amount_in_u128 * platform_fee_rate / 10000) as u128;
    let share_fee = (amount_in_u128 * share_fee_rate / 10000) as u128;
    amount_in_u128
        .checked_sub(protocol_fee)
        .unwrap()
        .checked_sub(platform_fee)
        .unwrap()
        .checked_sub(share_fee)
        .unwrap() as u64
}

pub fn get_amount_in(
    amount_out: u64,
    protocol_fee_rate: u128,
    platform_fee_rate: u128,
    share_fee_rate: u128,
    virtual_base: u128,
    virtual_quote: u128,
    real_base: u128,
    real_quote: u128,
    slippage_basis_points: u128,
) -> u64 {
    let amount_out_u128 = amount_out as u128;
    let bps = clamp_slippage_basis_points_u128(slippage_basis_points);

    // Consider slippage, actual required output amount is higher
    let amount_out_with_slippage = amount_out_u128 * 10000 / (10000 - bps);

    let input_reserve = virtual_quote.checked_add(real_quote).unwrap();
    let output_reserve = virtual_base.checked_sub(real_base).unwrap();

    // Reverse calculate using AMM formula: amount_in_net = (amount_out * input_reserve) / (output_reserve - amount_out)
    let numerator = amount_out_with_slippage.checked_mul(input_reserve).unwrap();
    let denominator = output_reserve.checked_sub(amount_out_with_slippage).unwrap();
    let amount_in_net = numerator.checked_div(denominator).unwrap();

    // Calculate total fee rate
    let total_fee_rate = protocol_fee_rate + platform_fee_rate + share_fee_rate;

    let amount_in = amount_in_net * 10000 / (10000 - total_fee_rate);

    amount_in as u64
}

pub fn get_amount_out(
    amount_in: u64,
    protocol_fee_rate: u128,
    platform_fee_rate: u128,
    share_fee_rate: u128,
    virtual_base: u128,
    virtual_quote: u128,
    real_base: u128,
    real_quote: u128,
    slippage_basis_points: u128,
) -> u64 {
    let amount_in_u128 = amount_in as u128;
    let bps = clamp_slippage_basis_points_u128(slippage_basis_points);
    let protocol_fee = (amount_in_u128 * protocol_fee_rate / 10000) as u128;
    let platform_fee = (amount_in_u128 * platform_fee_rate / 10000) as u128;
    let share_fee = (amount_in_u128 * share_fee_rate / 10000) as u128;
    let amount_in_net = amount_in_u128
        .checked_sub(protocol_fee)
        .unwrap()
        .checked_sub(platform_fee)
        .unwrap()
        .checked_sub(share_fee)
        .unwrap();
    let input_reserve = virtual_quote.checked_add(real_quote).unwrap();
    let output_reserve = virtual_base.checked_sub(real_base).unwrap();
    let numerator = amount_in_net.checked_mul(output_reserve).unwrap();
    let denominator = input_reserve.checked_add(amount_in_net).unwrap();
    let mut amount_out = numerator.checked_div(denominator).unwrap();

    amount_out = amount_out - (amount_out * bps) / 10000;
    amount_out as u64
}

pub fn get_pool_pda(base_mint: &Pubkey, quote_mint: &Pubkey) -> Option<Pubkey> {
    crate::common::fast_fn::get_cached_pda(
        crate::common::fast_fn::PdaCacheKey::BonkPool(*base_mint, *quote_mint),
        || {
            let seeds: &[&[u8]; 3] = &[seeds::POOL_SEED, base_mint.as_ref(), quote_mint.as_ref()];
            let program_id: &Pubkey = &accounts::BONK;
            let pda: Option<(Pubkey, u8)> = Pubkey::try_find_program_address(seeds, program_id);
            pda.map(|pubkey| pubkey.0)
        },
    )
}

pub fn get_vault_pda(pool_state: &Pubkey, mint: &Pubkey) -> Option<Pubkey> {
    crate::common::fast_fn::get_cached_pda(
        crate::common::fast_fn::PdaCacheKey::BonkVault(*pool_state, *mint),
        || {
            let seeds: &[&[u8]; 3] = &[seeds::POOL_VAULT_SEED, pool_state.as_ref(), mint.as_ref()];
            let program_id: &Pubkey = &accounts::BONK;
            let pda: Option<(Pubkey, u8)> = Pubkey::try_find_program_address(seeds, program_id);
            pda.map(|pubkey| pubkey.0)
        },
    )
}

pub fn get_platform_associated_account(platform_config: &Pubkey) -> Option<Pubkey> {
    get_platform_associated_account_for_quote(
        platform_config,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
    )
}

pub fn get_platform_associated_account_for_quote(
    platform_config: &Pubkey,
    quote_mint: &Pubkey,
) -> Option<Pubkey> {
    let seeds: &[&[u8]; 2] = &[platform_config.as_ref(), quote_mint.as_ref()];
    let program_id: &Pubkey = &accounts::BONK;
    let pda: Option<(Pubkey, u8)> = Pubkey::try_find_program_address(seeds, program_id);
    pda.map(|pubkey| pubkey.0)
}

pub fn get_creator_associated_account(creator: &Pubkey) -> Option<Pubkey> {
    get_creator_associated_account_for_quote(creator, &crate::constants::WSOL_TOKEN_ACCOUNT)
}

pub fn get_creator_associated_account_for_quote(
    creator: &Pubkey,
    quote_mint: &Pubkey,
) -> Option<Pubkey> {
    let seeds: &[&[u8]; 2] = &[creator.as_ref(), quote_mint.as_ref()];
    let program_id: &Pubkey = &accounts::BONK;
    let pda: Option<(Pubkey, u8)> = Pubkey::try_find_program_address(seeds, program_id);
    pda.map(|pubkey| pubkey.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::calc::common::MAX_SLIPPAGE_BASIS_POINTS;
    use solana_sdk::pubkey;

    const VIRTUAL_BASE: u128 = 1_073_025_605_596_382;
    const VIRTUAL_QUOTE: u128 = 30_000_852_951;

    #[test]
    fn stonkfun_reward_quote_accounts_match_mainnet() {
        let quote_mint = pubkey!("CARDSccUMFKoPRZxt5vt3ksUbxEFEcnZ3H2pd3dKxYjp");
        let creator = pubkey!("DwhaceCnV6R2U6e5FfCcrDiYXtZr7s7KjUzV3swWazm6");

        assert_eq!(
            get_platform_associated_account_for_quote(
                &accounts::STONKFUN_REWARD_PLATFORM_CONFIG,
                &quote_mint,
            )
            .unwrap(),
            pubkey!("hD6YgNjkVtaw5snL74P1VmUrQtGUoGAkgwHgtsWsj1H")
        );
        assert_eq!(
            get_creator_associated_account_for_quote(&creator, &quote_mint).unwrap(),
            pubkey!("GZrRchHGgeZXRjv2wEfCyUHfiChc8NNjxwJJijokWBnp")
        );
    }

    #[test]
    fn get_amount_in_at_10000_bps_does_not_divide_by_zero() {
        let amount = get_amount_in(
            1_000_000,
            accounts::PROTOCOL_FEE_RATE,
            accounts::PLATFORM_FEE_RATE,
            accounts::SHARE_FEE_RATE,
            VIRTUAL_BASE,
            VIRTUAL_QUOTE,
            0,
            0,
            10_000,
        );
        let clamped = get_amount_in(
            1_000_000,
            accounts::PROTOCOL_FEE_RATE,
            accounts::PLATFORM_FEE_RATE,
            accounts::SHARE_FEE_RATE,
            VIRTUAL_BASE,
            VIRTUAL_QUOTE,
            0,
            0,
            MAX_SLIPPAGE_BASIS_POINTS as u128,
        );
        assert_eq!(amount, clamped);
        assert!(amount > 0);
    }

    #[test]
    fn get_amount_out_at_10000_bps_does_not_zero_min_out() {
        let amount = get_amount_out(
            1_000_000,
            accounts::PROTOCOL_FEE_RATE,
            accounts::PLATFORM_FEE_RATE,
            accounts::SHARE_FEE_RATE,
            VIRTUAL_BASE,
            VIRTUAL_QUOTE,
            0,
            0,
            10_000,
        );
        let clamped = get_amount_out(
            1_000_000,
            accounts::PROTOCOL_FEE_RATE,
            accounts::PLATFORM_FEE_RATE,
            accounts::SHARE_FEE_RATE,
            VIRTUAL_BASE,
            VIRTUAL_QUOTE,
            0,
            0,
            MAX_SLIPPAGE_BASIS_POINTS as u128,
        );
        assert_eq!(amount, clamped);
        assert!(amount > 0);
    }
}
