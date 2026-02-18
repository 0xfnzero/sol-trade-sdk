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
}

pub const POOL_SIZE: usize = 1 + 2 + 32 * 6 + 8 + 32 + 1 + 1;

pub fn pool_decode(data: &[u8]) -> Option<Pool> {
    if data.len() < POOL_SIZE {
        return None;
    }
    borsh::from_slice::<Pool>(&data[..POOL_SIZE]).ok()
}
