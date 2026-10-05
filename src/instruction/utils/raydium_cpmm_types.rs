use borsh::BorshDeserialize;
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, BorshDeserialize)]
pub struct PoolState {
    pub amm_config: Pubkey,
    pub pool_creator: Pubkey,
    pub token0_vault: Pubkey,
    pub token1_vault: Pubkey,
    pub lp_mint: Pubkey,
    pub token0_mint: Pubkey,
    pub token1_mint: Pubkey,
    pub token0_program: Pubkey,
    pub token1_program: Pubkey,
    pub observation_key: Pubkey,
    pub auth_bump: u8,
    pub status: u8,
    pub lp_mint_decimals: u8,
    pub mint0_decimals: u8,
    pub mint1_decimals: u8,
    pub lp_supply: u64,
    pub protocol_fees_token0: u64,
    pub protocol_fees_token1: u64,
    pub fund_fees_token0: u64,
    pub fund_fees_token1: u64,
    pub open_time: u64,
    pub recent_epoch: u64,
    pub creator_fee_on: u8,
    pub enable_creator_fee: bool,
    pub padding1: [u8; 6],
    pub creator_fees_token0: u64,
    pub creator_fees_token1: u64,
    pub padding: [u64; 28],
}

pub const POOL_STATE_SIZE: usize = 629;
pub const POOL_STATE_DISCRIMINATOR: [u8; 8] = [247, 237, 227, 245, 215, 195, 222, 70];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, BorshDeserialize)]
pub struct AmmConfig {
    pub bump: u8,
    pub disable_create_pool: bool,
    pub index: u16,
    pub trade_fee_rate: u64,
    pub protocol_fee_rate: u64,
    pub fund_fee_rate: u64,
    pub create_pool_fee: u64,
    pub protocol_owner: Pubkey,
    pub fund_owner: Pubkey,
    pub creator_fee_rate: u64,
    /// Protocol share of accrued creator fees, in millionths; overridden by CreatorFeeShare.
    pub creator_fee_share_rate: u64,
    pub padding: [u64; 14],
}

pub const AMM_CONFIG_SIZE: usize = 228;
pub const AMM_CONFIG_DISCRIMINATOR: [u8; 8] = [218, 244, 33, 104, 203, 203, 43, 111];

pub fn amm_config_decode(data: &[u8]) -> Option<AmmConfig> {
    if data.len() < AMM_CONFIG_SIZE {
        return None;
    }
    borsh::from_slice::<AmmConfig>(&data[..AMM_CONFIG_SIZE]).ok()
}

pub fn pool_state_decode(data: &[u8]) -> Option<PoolState> {
    if data.len() < POOL_STATE_SIZE {
        return None;
    }
    borsh::from_slice::<PoolState>(&data[..POOL_STATE_SIZE]).ok()
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, BorshDeserialize)]
pub struct CreatorFeeShare {
    pub bump: u8,
    pub creator: Pubkey,
    pub amm_config: Pubkey,
    pub share_rate: u64,
    pub padding: [u64; 8],
}

pub const CREATOR_FEE_SHARE_SIZE: usize = 137;
pub const CREATOR_FEE_SHARE_DISCRIMINATOR: [u8; 8] = [30, 235, 98, 252, 26, 197, 66, 86];

/// Decode the full Anchor account, including discriminator.
pub fn creator_fee_share_decode(data: &[u8]) -> Option<CreatorFeeShare> {
    if data.len() < 8 + CREATOR_FEE_SHARE_SIZE || data[..8] != CREATOR_FEE_SHARE_DISCRIMINATOR {
        return None;
    }
    borsh::from_slice(&data[8..8 + CREATOR_FEE_SHARE_SIZE]).ok()
}
