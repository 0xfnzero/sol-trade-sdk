use borsh::BorshDeserialize;
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, BorshDeserialize)]
pub struct BaseFeeStruct {
    pub cliff_fee_numerator: u64,
    pub fee_scheduler_mode: u8,
    pub padding_0: [u8; 5],
    pub number_of_period: u16,
    pub period_frequency: u64,
    pub reduction_factor: u64,
    pub padding_1: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, BorshDeserialize)]
pub struct DynamicFeeStruct {
    pub initialized: u8,
    pub padding: [u8; 7],
    pub max_volatility_accumulator: u32,
    pub variable_fee_control: u32,
    pub bin_step: u16,
    pub filter_period: u16,
    pub decay_period: u16,
    pub reduction_factor: u16,
    pub last_update_timestamp: u64,
    pub bin_step_u128: u128,
    pub sqrt_price_reference: u128,
    pub volatility_accumulator: u128,
    pub volatility_reference: u128,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, BorshDeserialize)]
pub struct PoolFeesStruct {
    /// Current DAMM v2 fields projected from previously reserved wire bytes.
    #[borsh(skip)]
    #[serde(default)]
    pub compounding_fee_bps: u16,
    #[borsh(skip)]
    #[serde(default)]
    pub init_sqrt_price: u128,
    pub base_fee: BaseFeeStruct,
    pub protocol_fee_percent: u8,
    pub partner_fee_percent: u8,
    pub referral_fee_percent: u8,
    pub padding_0: [u8; 5],
    pub dynamic_fee: DynamicFeeStruct,
    pub padding_1: [u64; 2],
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, BorshDeserialize)]
pub struct PoolMetrics {
    pub total_lp_a_fee: u128,
    pub total_lp_b_fee: u128,
    pub total_protocol_a_fee: u64,
    pub total_protocol_b_fee: u64,
    pub total_partner_a_fee: u64,
    pub total_partner_b_fee: u64,
    pub total_position: u64,
    pub padding: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, BorshDeserialize)]
pub struct RewardInfo {
    pub initialized: u8,
    pub reward_token_flag: u8,
    pub padding_0: [u8; 6],
    pub padding_1: [u8; 8],
    pub mint: Pubkey,
    pub vault: Pubkey,
    pub funder: Pubkey,
    pub reward_duration: u64,
    pub reward_duration_end: u64,
    pub reward_rate: u128,
    pub reward_per_token_stored: [u8; 32],
    pub last_update_time: u64,
    pub cumulative_seconds_with_empty_liquidity_reward: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, BorshDeserialize)]
pub struct Pool {
    /// Current fields; old padding/partner fields below remain raw compatibility overlays.
    #[borsh(skip)]
    #[serde(default)]
    pub dead_liquidity_fee_checkpoint: u64,
    #[borsh(skip)]
    #[serde(default)]
    pub fee_version: u8,
    #[borsh(skip)]
    #[serde(default)]
    pub creator: Pubkey,
    #[borsh(skip)]
    #[serde(default)]
    pub token_a_amount: u64,
    #[borsh(skip)]
    #[serde(default)]
    pub token_b_amount: u64,
    #[borsh(skip)]
    #[serde(default)]
    pub layout_version: u8,
    pub pool_fees: PoolFeesStruct,
    pub token_a_mint: Pubkey,
    pub token_b_mint: Pubkey,
    pub token_a_vault: Pubkey,
    pub token_b_vault: Pubkey,
    pub whitelisted_vault: Pubkey,
    pub partner: Pubkey,
    pub liquidity: u128,
    pub padding: u128,
    pub protocol_a_fee: u64,
    pub protocol_b_fee: u64,
    pub partner_a_fee: u64,
    pub partner_b_fee: u64,
    pub sqrt_min_price: u128,
    pub sqrt_max_price: u128,
    pub sqrt_price: u128,
    pub activation_point: u64,
    pub activation_type: u8,
    pub pool_status: u8,
    pub token_a_flag: u8,
    pub token_b_flag: u8,
    pub collect_fee_mode: u8,
    pub pool_type: u8,
    pub padding_0: [u8; 2],
    pub fee_a_per_liquidity: [u8; 32],
    pub fee_b_per_liquidity: [u8; 32],
    pub permanent_lock_liquidity: u128,
    pub metrics: PoolMetrics,
    pub padding_1: [u64; 10],
    pub reward_infos: [RewardInfo; 2],
}

pub const POOL_SIZE: usize = 1104;

pub fn pool_decode(data: &[u8]) -> Option<Pool> {
    if data.len() < POOL_SIZE {
        return None;
    }
    let mut pool = borsh::from_slice::<Pool>(&data[..POOL_SIZE]).ok()?;
    pool.pool_fees.compounding_fee_bps = u16::from_le_bytes(data[46..48].try_into().ok()?);
    pool.pool_fees.init_sqrt_price = u128::from_le_bytes(data[144..160].try_into().ok()?);
    pool.dead_liquidity_fee_checkpoint = u64::from_le_bytes(data[400..408].try_into().ok()?);
    pool.fee_version = data[478];
    pool.creator = Pubkey::new_from_array(data[640..672].try_into().ok()?);
    pool.token_a_amount = u64::from_le_bytes(data[672..680].try_into().ok()?);
    pool.token_b_amount = u64::from_le_bytes(data[680..688].try_into().ok()?);
    pool.layout_version = data[688];
    Some(pool)
}

/// Anchor Pool discriminator from current official cp_amm IDL.
pub const POOL_DISCRIMINATOR: [u8; 8] = [241, 154, 109, 4, 17, 177, 109, 188];

/// Decode a full account rather than an unchecked 8-byte-prefixed payload.
pub fn pool_account_decode(data: &[u8]) -> Option<Pool> {
    if data.get(..8)? != POOL_DISCRIMINATOR {
        return None;
    }
    pool_decode(data.get(8..)?)
}

#[cfg(test)]
mod current_layout_tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};
    #[test]
    fn official_current_pool_fields_and_invalid_account_headers() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/damm_v2_current.json"))
                .unwrap();
        let data = STANDARD.decode(fixture["payload"].as_str().unwrap()).unwrap();
        let p = pool_decode(&data).unwrap();
        assert_eq!(p.pool_fees.compounding_fee_bps, 321);
        assert_eq!(p.pool_fees.init_sqrt_price, (1u128 << 100) + 123);
        assert_eq!(p.dead_liquidity_fee_checkpoint, 987654321);
        assert_eq!((p.fee_version, p.layout_version), (1, 1));
        assert_eq!(p.creator.to_bytes(), std::array::from_fn(|i| i as u8));
        assert_eq!((p.token_a_amount, p.token_b_amount), (9007199254740993, 9007199254740995));
        assert_eq!(p.sqrt_price, (1u128 << 64) + 1234);
        let mut account = POOL_DISCRIMINATOR.to_vec();
        account.extend(&data);
        assert!(pool_account_decode(&account).is_some());
        account[0] ^= 1;
        assert!(pool_account_decode(&account).is_none());
        account[0] ^= 1;
        for len in 0..8 + POOL_SIZE {
            assert!(pool_account_decode(&account[..len]).is_none());
        }
    }
}
