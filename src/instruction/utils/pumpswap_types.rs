use borsh::BorshDeserialize;
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;

pub const POOL_DISCRIMINATOR: [u8; 8] = [241, 154, 109, 4, 17, 177, 109, 188];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, BorshDeserialize)]
pub struct Pool {
    #[borsh(skip)]
    #[serde(default)]
    pub protocol_fees: u64,
    #[borsh(skip)]
    #[serde(default)]
    pub creator_fees: u64,
    pub pool_bump: u8,
    pub index: u16,
    pub creator: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub lp_mint: Pubkey,
    pub pool_base_token_account: Pubkey,
    pub pool_quote_token_account: Pubkey,
    pub lp_supply: u64,
    pub coin_creator: Pubkey,
    pub is_mayhem_mode: bool,
    /// Whether this pool's coin has cashback enabled
    pub is_cashback_coin: bool,
    /// Virtual quote reserves appended to the Pool account.
    ///
    /// Quotes must use `quote_vault_balance + virtual_quote_reserves`.
    pub virtual_quote_reserves: i128,
    #[serde(default)]
    pub creator_fee_bps: u64,
    #[serde(default)]
    pub can_edit_creator_fee: bool,
    #[serde(default)]
    pub is_holder_reward: bool,
}

/// Minimum Borsh payload length for the current Pool layout, excluding the
/// 8-byte Anchor account discriminator.
pub const POOL_BOOST_SIZE: usize = 1 + 2 + 32 * 6 + 8 + 32 + 1 + 1 + 16;
pub const POOL_CREATOR_FEE_SIZE: usize = POOL_BOOST_SIZE + 9;
pub const POOL_SIZE: usize = POOL_CREATOR_FEE_SIZE + 1;
const LEGACY_POOL_FIELDS_SIZE: usize = 1 + 2 + 32 * 6 + 8 + 32 + 1 + 1;
/// Legacy Pool accounts were allocated with seven trailing padding bytes.
pub const LEGACY_POOL_SIZE: usize = LEGACY_POOL_FIELDS_SIZE + 7;

#[derive(BorshDeserialize)]
struct LegacyPool {
    pool_bump: u8,
    index: u16,
    creator: Pubkey,
    base_mint: Pubkey,
    quote_mint: Pubkey,
    lp_mint: Pubkey,
    pool_base_token_account: Pubkey,
    pool_quote_token_account: Pubkey,
    lp_supply: u64,
    coin_creator: Pubkey,
    is_mayhem_mode: bool,
    is_cashback_coin: bool,
}

impl From<LegacyPool> for Pool {
    fn from(pool: LegacyPool) -> Self {
        Self {
            pool_bump: pool.pool_bump,
            index: pool.index,
            creator: pool.creator,
            base_mint: pool.base_mint,
            quote_mint: pool.quote_mint,
            lp_mint: pool.lp_mint,
            pool_base_token_account: pool.pool_base_token_account,
            pool_quote_token_account: pool.pool_quote_token_account,
            lp_supply: pool.lp_supply,
            coin_creator: pool.coin_creator,
            is_mayhem_mode: pool.is_mayhem_mode,
            is_cashback_coin: pool.is_cashback_coin,
            virtual_quote_reserves: 0,
            creator_fee_bps: 0,
            can_edit_creator_fee: false,
            is_holder_reward: false,
            protocol_fees: 0,
            creator_fees: 0,
        }
    }
}

impl Pool {
    pub fn is_boosted(&self) -> bool {
        self.virtual_quote_reserves
            .checked_add(self.protocol_fees as i128)
            .and_then(|v| v.checked_add(self.creator_fees as i128))
            .is_some_and(|v| v != 0)
    }
}
pub fn pool_decode(data: &[u8]) -> Option<Pool> {
    if data.len() >= POOL_SIZE {
        let mut pool = borsh::from_slice::<Pool>(&data[..POOL_SIZE]).ok()?;
        pool.protocol_fees =
            data.get(263..271).map(|v| u64::from_le_bytes(v.try_into().unwrap())).unwrap_or(0);
        pool.creator_fees =
            data.get(271..279).map(|v| u64::from_le_bytes(v.try_into().unwrap())).unwrap_or(0);
        return Some(pool);
    }
    if ![LEGACY_POOL_SIZE, POOL_BOOST_SIZE, POOL_CREATOR_FEE_SIZE].contains(&data.len()) {
        return None;
    }
    let mut pool: Pool =
        borsh::from_slice::<LegacyPool>(&data[..LEGACY_POOL_FIELDS_SIZE]).ok()?.into();
    if data.len() >= POOL_BOOST_SIZE {
        pool.virtual_quote_reserves =
            i128::from_le_bytes(data[LEGACY_POOL_FIELDS_SIZE..POOL_BOOST_SIZE].try_into().ok()?);
    }
    if data.len() >= POOL_CREATOR_FEE_SIZE {
        pool.creator_fee_bps =
            u64::from_le_bytes(data[POOL_BOOST_SIZE..POOL_BOOST_SIZE + 8].try_into().ok()?);
        pool.can_edit_creator_fee = match data[POOL_BOOST_SIZE + 8] {
            0 => false,
            1 => true,
            _ => return None,
        };
    }
    Some(pool)
}

/// Compute the quote reserves used by PumpSwap pricing.
///
/// Zero is a valid reserve snapshot; trading quote functions reject depleted pools.
/// Returns `None` when the signed sum is negative or cannot fit in a `u64`.
#[inline]
pub fn effective_quote_reserves(
    quote_vault_balance: u64,
    virtual_quote_reserves: i128,
) -> Option<u64> {
    i128::from(quote_vault_balance)
        .checked_add(virtual_quote_reserves)
        .and_then(|reserves| u64::try_from(reserves).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool_payload(virtual_quote_reserves: i128) -> Vec<u8> {
        let mut data = Vec::with_capacity(POOL_SIZE);
        data.push(7);
        data.extend_from_slice(&42u16.to_le_bytes());
        for seed in 1..=6 {
            data.extend_from_slice(Pubkey::new_from_array([seed; 32]).as_ref());
        }
        data.extend_from_slice(&123_456u64.to_le_bytes());
        data.extend_from_slice(Pubkey::new_from_array([7; 32]).as_ref());
        data.push(1);
        data.push(0);
        data.extend_from_slice(&virtual_quote_reserves.to_le_bytes());
        data.extend_from_slice(&[0; 10]);
        data
    }

    #[test]
    fn decodes_current_pool_virtual_quote_reserves() {
        let pool = pool_decode(&pool_payload(987_654_321)).unwrap();

        assert_eq!(pool.virtual_quote_reserves, 987_654_321);
        assert!(pool.is_mayhem_mode);
        assert!(!pool.is_cashback_coin);
    }

    #[test]
    fn decodes_latest_pool_fee_fields_and_retains_boost_compatibility() {
        let mut data = pool_payload(-500);
        data[POOL_BOOST_SIZE..POOL_BOOST_SIZE + 8].copy_from_slice(&200u64.to_le_bytes());
        data[POOL_BOOST_SIZE + 8] = 1;
        data[POOL_CREATOR_FEE_SIZE] = 1;
        let current = pool_decode(&data).unwrap();
        assert_eq!(current.virtual_quote_reserves, -500);
        assert_eq!(current.creator_fee_bps, 200);
        assert!(current.can_edit_creator_fee && current.is_holder_reward);
        let previous = pool_decode(&data[..POOL_CREATOR_FEE_SIZE]).unwrap();
        assert_eq!(previous.creator_fee_bps, 200);
        assert!(!previous.is_holder_reward);
        let boost = pool_decode(&data[..POOL_BOOST_SIZE]).unwrap();
        assert_eq!(boost.virtual_quote_reserves, -500);
        assert_eq!(boost.creator_fee_bps, 0);
    }

    #[test]
    fn decodes_legacy_pool_with_zero_virtual_quote_reserves() {
        let mut data = pool_payload(0);
        data.truncate(LEGACY_POOL_FIELDS_SIZE);
        data.extend_from_slice(&[0; 7]);

        let pool = pool_decode(&data).unwrap();
        assert_eq!(pool.virtual_quote_reserves, 0);
    }

    #[test]
    fn rejects_partial_current_pool_layout() {
        let current = pool_payload(987_654_321);
        for len in ((LEGACY_POOL_SIZE + 1)..POOL_BOOST_SIZE)
            .chain((POOL_BOOST_SIZE + 1)..POOL_CREATOR_FEE_SIZE)
        {
            assert!(pool_decode(&current[..len]).is_none(), "accepted body length {len}");
        }
    }

    #[test]
    fn effective_reserves_support_signed_virtual_amounts_and_reject_invalid_sums() {
        assert_eq!(effective_quote_reserves(1_000, 250), Some(1_250));
        assert_eq!(effective_quote_reserves(1_000, -250), Some(750));
        assert_eq!(effective_quote_reserves(1_000, -1_000), Some(0));
        assert_eq!(effective_quote_reserves(100, -101), None);
        assert_eq!(effective_quote_reserves(u64::MAX, 1), None);
    }
}
