//! `AmmConfig` fields a swap reads (`programs/amm/src/states/config.rs`).

use super::error::{ErrorCode, Result};

pub const FEE_RATE_DENOMINATOR_VALUE: u32 = 1_000_000;

/// Anchor discriminator of `AmmConfig`.
pub const AMM_CONFIG_DISCRIMINATOR: [u8; 8] = [218, 244, 33, 104, 203, 203, 43, 111];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AmmConfig {
    pub protocol_fee_rate: u32,
    /// Base trade fee, in hundredths of a bip (`FEE_RATE_DENOMINATOR_VALUE`).
    pub trade_fee_rate: u32,
    pub tick_spacing: u16,
    pub fund_fee_rate: u32,
}

impl AmmConfig {
    /// Layout: discriminator, bump, index u16, owner, protocol_fee_rate u32 @43,
    /// trade_fee_rate u32 @47, tick_spacing u16 @51, fund_fee_rate u32 @53.
    pub fn decode(data: &[u8]) -> Result<Self> {
        if data.len() < 57 || data[..8] != AMM_CONFIG_DISCRIMINATOR {
            return Err(ErrorCode::InvalidAccountData);
        }
        let u32_at =
            |offset: usize| u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
        Ok(Self {
            protocol_fee_rate: u32_at(43),
            trade_fee_rate: u32_at(47),
            tick_spacing: u16::from_le_bytes(data[51..53].try_into().unwrap()),
            fund_fee_rate: u32_at(53),
        })
    }
}
