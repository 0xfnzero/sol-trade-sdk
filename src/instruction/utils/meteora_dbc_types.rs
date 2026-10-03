//! Meteora Dynamic Bonding Curve accounts, read from their zero-copy layouts.
//!
//! A pool is a `VirtualPool` or, for a base token with a transfer hook, a
//! `TransferHookPool`; both hold the same `PoolState`. Its config is a
//! `PoolConfig` or a `ConfigWithTransferHook`, a `PoolConfig` followed by the
//! hook program.

use solana_sdk::pubkey::Pubkey;

pub const VIRTUAL_POOL_DISCRIMINATOR: [u8; 8] = [213, 224, 5, 209, 98, 69, 119, 92];
pub const TRANSFER_HOOK_POOL_DISCRIMINATOR: [u8; 8] = [237, 219, 184, 23, 42, 189, 169, 35];
pub const POOL_CONFIG_DISCRIMINATOR: [u8; 8] = [26, 108, 14, 123, 116, 230, 129, 43];
pub const CONFIG_WITH_TRANSFER_HOOK_DISCRIMINATOR: [u8; 8] = [40, 220, 194, 251, 41, 199, 123, 253];

const DISCRIMINATOR_LEN: usize = 8;
const POOL_STATE_LEN: usize = 416;
const POOL_CONFIG_LEN: usize = 1040;
/// Curve points a config has room for.
const CURVE_POINTS: usize = 20;

/// `PoolConfig::collect_fee_mode`: fees are taken in the quote token.
pub const COLLECT_FEE_QUOTE_TOKEN: u8 = 0;
/// `PoolConfig::collect_fee_mode`: fees are taken in the token the swap pays out.
pub const COLLECT_FEE_OUTPUT_TOKEN: u8 = 1;

/// `PoolConfig::activation_type`: points are slots.
pub const ACTIVATION_SLOT: u8 = 0;
/// `PoolConfig::activation_type`: points are unix timestamps.
pub const ACTIVATION_TIMESTAMP: u8 = 1;

/// `BaseFeeConfig::base_fee_mode`.
pub const BASE_FEE_SCHEDULER_LINEAR: u8 = 0;
pub const BASE_FEE_SCHEDULER_EXPONENTIAL: u8 = 1;
pub const BASE_FEE_RATE_LIMITER: u8 = 2;

/// `PoolConfig::migration_option`: where a completed curve migrates.
pub const MIGRATION_DAMM_V1: u8 = 0;
pub const MIGRATION_DAMM_V2: u8 = 1;

/// The volatility a pool's dynamic fee is priced from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DbcVolatilityTracker {
    pub last_update_timestamp: u64,
    pub sqrt_price_reference: u128,
    pub volatility_accumulator: u128,
    pub volatility_reference: u128,
}

/// A pool's state (`PoolState`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DbcPool {
    /// A `TransferHookPool`, traded with `swap2_with_transfer_hook`.
    pub transfer_hook: bool,
    pub volatility: DbcVolatilityTracker,
    pub config: Pubkey,
    pub creator: Pubkey,
    pub base_mint: Pubkey,
    pub base_vault: Pubkey,
    pub quote_vault: Pubkey,
    pub base_reserve: u64,
    pub quote_reserve: u64,
    pub sqrt_price: u128,
    pub activation_point: u64,
    /// 0 SPL Token, 1 Token-2022.
    pub pool_type: u8,
    pub is_migrated: bool,
    /// 0 trading on the curve, then the steps of its migration.
    pub migration_progress: u8,
    pub finish_curve_timestamp: u64,
    pub has_swap: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DbcBaseFee {
    pub cliff_fee_numerator: u64,
    /// Scheduler: number of periods. Rate limiter: fee increment in basis points.
    pub first_factor: u16,
    /// Scheduler: period length in points. Rate limiter: its duration in points.
    pub second_factor: u64,
    /// Scheduler: reduction per period. Rate limiter: reference amount.
    pub third_factor: u64,
    pub base_fee_mode: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DbcDynamicFee {
    pub initialized: bool,
    pub max_volatility_accumulator: u32,
    pub variable_fee_control: u32,
    pub bin_step: u16,
    pub filter_period: u16,
    pub decay_period: u16,
    pub reduction_factor: u16,
    pub bin_step_u128: u128,
}

/// The curve holds `liquidity` from the point before, or the start price, up
/// to `sqrt_price`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DbcCurvePoint {
    pub sqrt_price: u128,
    pub liquidity: u128,
}

/// What a swap reads of a pool's config (`PoolConfig`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DbcConfig {
    pub quote_mint: Pubkey,
    pub base_fee: DbcBaseFee,
    pub dynamic_fee: DbcDynamicFee,
    pub collect_fee_mode: u8,
    pub migration_option: u8,
    pub activation_type: u8,
    pub token_decimal: u8,
    /// 0 SPL Token, 1 Token-2022.
    pub token_type: u8,
    /// Which pool fee the migrated pool takes: 0 to 5 a fixed fee, 6 the
    /// config's own.
    pub migration_fee_option: u8,
    /// The quote reserve at which the curve completes.
    pub migration_quote_threshold: u64,
    /// The price the curve completes at; buys stop there.
    pub migration_sqrt_price: u128,
    pub sqrt_start_price: u128,
    /// The curve's points, lowest price first.
    pub curve: Vec<DbcCurvePoint>,
    /// The base token's hook program, of a `ConfigWithTransferHook`.
    pub transfer_hook_program: Option<Pubkey>,
}

fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(data.get(offset..offset + 2)?.try_into().ok()?))
}

fn u32_at(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(offset..offset + 4)?.try_into().ok()?))
}

fn u64_at(data: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(data.get(offset..offset + 8)?.try_into().ok()?))
}

fn u128_at(data: &[u8], offset: usize) -> Option<u128> {
    Some(u128::from_le_bytes(data.get(offset..offset + 16)?.try_into().ok()?))
}

fn pubkey_at(data: &[u8], offset: usize) -> Option<Pubkey> {
    Some(Pubkey::new_from_array(data.get(offset..offset + 32)?.try_into().ok()?))
}

/// Decodes a pool account, discriminator included; `None` when it is not a
/// DBC pool.
pub fn pool_decode(account_data: &[u8]) -> Option<DbcPool> {
    let transfer_hook = match account_data.get(..DISCRIMINATOR_LEN)? {
        discriminator if discriminator == VIRTUAL_POOL_DISCRIMINATOR => false,
        discriminator if discriminator == TRANSFER_HOOK_POOL_DISCRIMINATOR => true,
        _ => return None,
    };
    let data = account_data.get(DISCRIMINATOR_LEN..DISCRIMINATOR_LEN + POOL_STATE_LEN)?;
    Some(DbcPool {
        transfer_hook,
        volatility: DbcVolatilityTracker {
            last_update_timestamp: u64_at(data, 0)?,
            sqrt_price_reference: u128_at(data, 16)?,
            volatility_accumulator: u128_at(data, 32)?,
            volatility_reference: u128_at(data, 48)?,
        },
        config: pubkey_at(data, 64)?,
        creator: pubkey_at(data, 96)?,
        base_mint: pubkey_at(data, 128)?,
        base_vault: pubkey_at(data, 160)?,
        quote_vault: pubkey_at(data, 192)?,
        base_reserve: u64_at(data, 224)?,
        quote_reserve: u64_at(data, 232)?,
        sqrt_price: u128_at(data, 272)?,
        activation_point: u64_at(data, 288)?,
        pool_type: *data.get(296)?,
        is_migrated: *data.get(297)? != 0,
        migration_progress: *data.get(300)?,
        finish_curve_timestamp: u64_at(data, 336)?,
        has_swap: *data.get(362)? != 0,
    })
}

/// Decodes a config account, discriminator included; `None` when it is not a
/// DBC config.
pub fn config_decode(account_data: &[u8]) -> Option<DbcConfig> {
    let with_transfer_hook = match account_data.get(..DISCRIMINATOR_LEN)? {
        discriminator if discriminator == POOL_CONFIG_DISCRIMINATOR => false,
        discriminator if discriminator == CONFIG_WITH_TRANSFER_HOOK_DISCRIMINATOR => true,
        _ => return None,
    };
    let data = account_data.get(DISCRIMINATOR_LEN..DISCRIMINATOR_LEN + POOL_CONFIG_LEN)?;
    let mut curve = Vec::new();
    for index in 0..CURVE_POINTS {
        let offset = 400 + 32 * index;
        let point = DbcCurvePoint {
            sqrt_price: u128_at(data, offset)?,
            liquidity: u128_at(data, offset + 16)?,
        };
        // Points fill the array from the front.
        if point.sqrt_price == 0 || point.liquidity == 0 {
            break;
        }
        curve.push(point);
    }
    let transfer_hook_program = if with_transfer_hook {
        Some(pubkey_at(account_data, DISCRIMINATOR_LEN + POOL_CONFIG_LEN)?)
    } else {
        None
    };
    Some(DbcConfig {
        quote_mint: pubkey_at(data, 0)?,
        base_fee: DbcBaseFee {
            cliff_fee_numerator: u64_at(data, 96)?,
            second_factor: u64_at(data, 104)?,
            third_factor: u64_at(data, 112)?,
            first_factor: u16_at(data, 120)?,
            base_fee_mode: *data.get(122)?,
        },
        dynamic_fee: DbcDynamicFee {
            initialized: *data.get(128)? != 0,
            max_volatility_accumulator: u32_at(data, 136)?,
            variable_fee_control: u32_at(data, 140)?,
            bin_step: u16_at(data, 144)?,
            filter_period: u16_at(data, 146)?,
            decay_period: u16_at(data, 148)?,
            reduction_factor: u16_at(data, 150)?,
            bin_step_u128: u128_at(data, 160)?,
        },
        collect_fee_mode: *data.get(224)?,
        migration_option: *data.get(225)?,
        activation_type: *data.get(226)?,
        token_decimal: *data.get(227)?,
        token_type: *data.get(229)?,
        migration_fee_option: *data.get(235)?,
        migration_quote_threshold: u64_at(data, 256)?,
        migration_sqrt_price: u128_at(data, 272)?,
        sqrt_start_price: u128_at(data, 384)?,
        curve,
        transfer_hook_program,
    })
}
