//! Pump.fun bonding-curve `buy` / `buy_exact_quote_in` / `sell`
//! instruction data stack encoding. The public helper names are version-neutral;
//! [`PumpFunIxVersion`] selects the legacy or V2 on-chain discriminator.
//!
//! Legacy `buy` / `buy_exact_sol_in` 与 `@pump-fun/pump-sdk` 对齐：`OptionBool` 是单字段
//! struct（TypeScript 传 `[true]`），在 ix 参数中为 1 字节 bool，共 25 字节 ix data。
//! `*_v2` 指令无 `track_volume` 字节（见 [pump-public-docs](https://github.com/pump-fun/pump-public-docs)）。

use crate::instruction::utils::pumpfun::{
    BUY_DISCRIMINATOR, BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR, BUY_EXACT_SOL_IN_DISCRIMINATOR,
    BUY_V2_DISCRIMINATOR, SELL_DISCRIMINATOR, SELL_V2_DISCRIMINATOR,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PumpFunIxVersion {
    Legacy { track_volume: u8 },
    V2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PumpFunIxData {
    Bytes25([u8; 25]),
    Bytes24([u8; 24]),
}

impl PumpFunIxData {
    #[inline(always)]
    pub(crate) fn as_slice(&self) -> &[u8] {
        match self {
            Self::Bytes25(data) => data,
            Self::Bytes24(data) => data,
        }
    }
}

#[inline(always)]
pub(crate) fn encode_pumpfun_buy_ix_data(
    token_amount: u64,
    max_quote_cost: u64,
    version: PumpFunIxVersion,
) -> PumpFunIxData {
    match version {
        PumpFunIxVersion::Legacy { track_volume } => {
            let mut d = [0u8; 25];
            d[..8].copy_from_slice(&BUY_DISCRIMINATOR);
            d[8..16].copy_from_slice(&token_amount.to_le_bytes());
            d[16..24].copy_from_slice(&max_quote_cost.to_le_bytes());
            d[24] = track_volume;
            PumpFunIxData::Bytes25(d)
        }
        PumpFunIxVersion::V2 => {
            let mut d = [0u8; 24];
            d[..8].copy_from_slice(&BUY_V2_DISCRIMINATOR);
            d[8..16].copy_from_slice(&token_amount.to_le_bytes());
            d[16..24].copy_from_slice(&max_quote_cost.to_le_bytes());
            PumpFunIxData::Bytes24(d)
        }
    }
}

#[inline(always)]
pub(crate) fn encode_pumpfun_buy_exact_quote_in_ix_data(
    spendable_quote_in: u64,
    min_tokens_out: u64,
    version: PumpFunIxVersion,
) -> PumpFunIxData {
    match version {
        PumpFunIxVersion::Legacy { track_volume } => {
            let mut d = [0u8; 25];
            d[..8].copy_from_slice(&BUY_EXACT_SOL_IN_DISCRIMINATOR);
            d[8..16].copy_from_slice(&spendable_quote_in.to_le_bytes());
            d[16..24].copy_from_slice(&min_tokens_out.to_le_bytes());
            d[24] = track_volume;
            PumpFunIxData::Bytes25(d)
        }
        PumpFunIxVersion::V2 => {
            let mut d = [0u8; 24];
            d[..8].copy_from_slice(&BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR);
            d[8..16].copy_from_slice(&spendable_quote_in.to_le_bytes());
            d[16..24].copy_from_slice(&min_tokens_out.to_le_bytes());
            PumpFunIxData::Bytes24(d)
        }
    }
}

#[inline(always)]
pub(crate) fn encode_pumpfun_sell_ix_data(
    token_amount: u64,
    min_quote_output: u64,
    version: PumpFunIxVersion,
) -> PumpFunIxData {
    let mut d = [0u8; 24];
    d[..8].copy_from_slice(match version {
        PumpFunIxVersion::Legacy { .. } => &SELL_DISCRIMINATOR,
        PumpFunIxVersion::V2 => &SELL_V2_DISCRIMINATOR,
    });
    d[8..16].copy_from_slice(&token_amount.to_le_bytes());
    d[16..24].copy_from_slice(&min_quote_output.to_le_bytes());
    PumpFunIxData::Bytes24(d)
}
