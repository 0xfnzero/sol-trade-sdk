use borsh::BorshDeserialize;
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, BorshDeserialize)]
pub struct Pool {
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
}

/// Minimum Borsh payload length for the current Pool layout, excluding the
/// 8-byte Anchor account discriminator.
pub const POOL_SIZE: usize = 1 + 2 + 32 * 6 + 8 + 32 + 1 + 1 + 16;
const LEGACY_POOL_SIZE: usize = 1 + 2 + 32 * 6 + 8 + 32 + 1 + 1;

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
        }
    }
}

pub fn pool_decode(data: &[u8]) -> Option<Pool> {
    if data.len() >= POOL_SIZE {
        return borsh::from_slice::<Pool>(&data[..POOL_SIZE]).ok();
    }

    if data.len() >= LEGACY_POOL_SIZE {
        return borsh::from_slice::<LegacyPool>(&data[..LEGACY_POOL_SIZE]).ok().map(Into::into);
    }

    None
}

/// Compute the quote reserves used by PumpSwap pricing.
///
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
    fn decodes_legacy_pool_with_zero_virtual_quote_reserves() {
        let mut data = pool_payload(0);
        data.truncate(LEGACY_POOL_SIZE);
        data.extend_from_slice(&[0; 7]);

        let pool = pool_decode(&data).unwrap();
        assert_eq!(pool.virtual_quote_reserves, 0);
    }

    #[test]
    fn effective_reserves_support_signed_virtual_amounts_and_reject_invalid_sums() {
        assert_eq!(effective_quote_reserves(1_000, 250), Some(1_250));
        assert_eq!(effective_quote_reserves(1_000, -250), Some(750));
        assert_eq!(effective_quote_reserves(100, -101), None);
        assert_eq!(effective_quote_reserves(u64::MAX, 1), None);
    }
}
