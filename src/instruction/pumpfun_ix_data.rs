//! Pump.fun 曲线 **legacy** `buy` / `buy_exact_sol_in` / `sell` 与 **`buy_v2` / `sell_v2` / `buy_exact_quote_in_v2`**
//! 的 instruction data 栈上编码（热路径零堆分配）。
//!
//! Legacy `buy` / `buy_exact_sol_in` 与 `@pump-fun/pump-sdk` 对齐：`OptionBool` 在 ix 参数中为 **1 字节**。
//! `*_v2` 指令无 `track_volume` 字节（见 [pump-public-docs](https://github.com/pump-fun/pump-public-docs)）。

use crate::instruction::utils::pumpfun::{
    BUY_DISCRIMINATOR, BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR, BUY_EXACT_SOL_IN_DISCRIMINATOR,
    BUY_V2_DISCRIMINATOR, SELL_DISCRIMINATOR, SELL_V2_DISCRIMINATOR,
};

/// Legacy `buy`：`track_volume = true`（仅 legacy 指令使用）。
#[allow(dead_code)]
pub const TRACK_VOLUME_TRUE: u8 = 1;

/// Legacy `buy` ix data（保留供兼容 / 对照 IDL；热路径已改用 [`encode_pumpfun_buy_v2_ix_data`]）。
#[inline(always)]
#[allow(dead_code)]
pub fn encode_pumpfun_buy_ix_data(
    token_amount: u64,
    max_sol_cost: u64,
    track_volume: u8,
) -> [u8; 25] {
    let mut d = [0u8; 25];
    d[..8].copy_from_slice(&BUY_DISCRIMINATOR);
    d[8..16].copy_from_slice(&token_amount.to_le_bytes());
    d[16..24].copy_from_slice(&max_sol_cost.to_le_bytes());
    d[24] = track_volume;
    d
}

#[inline(always)]
#[allow(dead_code)]
pub fn encode_pumpfun_buy_exact_sol_in_ix_data(
    spendable_sol_in: u64,
    min_tokens_out: u64,
    track_volume: u8,
) -> [u8; 25] {
    let mut d = [0u8; 25];
    d[..8].copy_from_slice(&BUY_EXACT_SOL_IN_DISCRIMINATOR);
    d[8..16].copy_from_slice(&spendable_sol_in.to_le_bytes());
    d[16..24].copy_from_slice(&min_tokens_out.to_le_bytes());
    d[24] = track_volume;
    d
}

#[inline(always)]
#[allow(dead_code)]
pub fn encode_pumpfun_sell_ix_data(token_amount: u64, min_sol_output: u64) -> [u8; 24] {
    let mut d = [0u8; 24];
    d[..8].copy_from_slice(&SELL_DISCRIMINATOR);
    d[8..16].copy_from_slice(&token_amount.to_le_bytes());
    d[16..24].copy_from_slice(&min_sol_output.to_le_bytes());
    d
}

#[inline(always)]
pub fn encode_pumpfun_buy_v2_ix_data(amount: u64, max_sol_cost: u64) -> [u8; 24] {
    let mut d = [0u8; 24];
    d[..8].copy_from_slice(&BUY_V2_DISCRIMINATOR);
    d[8..16].copy_from_slice(&amount.to_le_bytes());
    d[16..24].copy_from_slice(&max_sol_cost.to_le_bytes());
    d
}

#[inline(always)]
pub fn encode_pumpfun_buy_exact_quote_in_v2_ix_data(
    spendable_quote_in: u64,
    min_tokens_out: u64,
) -> [u8; 24] {
    let mut d = [0u8; 24];
    d[..8].copy_from_slice(&BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR);
    d[8..16].copy_from_slice(&spendable_quote_in.to_le_bytes());
    d[16..24].copy_from_slice(&min_tokens_out.to_le_bytes());
    d
}

#[inline(always)]
pub fn encode_pumpfun_sell_v2_ix_data(token_amount: u64, min_sol_output: u64) -> [u8; 24] {
    let mut d = [0u8; 24];
    d[..8].copy_from_slice(&SELL_V2_DISCRIMINATOR);
    d[8..16].copy_from_slice(&token_amount.to_le_bytes());
    d[16..24].copy_from_slice(&min_sol_output.to_le_bytes());
    d
}
