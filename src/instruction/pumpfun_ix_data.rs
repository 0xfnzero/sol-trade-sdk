//! Pump.fun 曲线 `buy` / `buy_exact_sol_in` / `sell` 的 **instruction data** 栈上编码（热路径零堆分配）。
//!
//! 与 `@pump-fun/pump-sdk` Anchor `coder.instruction.encode` 对齐：`OptionBool` 在 ix 参数中为 **1 字节**。

use crate::instruction::utils::pumpfun::{
    BUY_DISCRIMINATOR, BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR, BUY_EXACT_SOL_IN_DISCRIMINATOR,
    BUY_V2_DISCRIMINATOR, SELL_DISCRIMINATOR, SELL_V2_DISCRIMINATOR,
};

/// 与官方 `getBuyInstructionInternal` 对齐：`OptionBool` 在 ix 中为 1 字节 option tag + 1 字节值。
/// `track_volume` 在调用侧组合为 `(1u8, cashback as u8)`（Option tag + value），共 26 字节 ix data。
pub const TRACK_VOLUME_OPTION_TAG: u8 = 1;

#[inline(always)]
pub fn encode_pumpfun_buy_ix_data(
    token_amount: u64,
    max_sol_cost: u64,
    track_volume_val: u8,
) -> [u8; 26] {
    let mut d = [0u8; 26];
    d[..8].copy_from_slice(&BUY_DISCRIMINATOR);
    d[8..16].copy_from_slice(&token_amount.to_le_bytes());
    d[16..24].copy_from_slice(&max_sol_cost.to_le_bytes());
    d[24] = TRACK_VOLUME_OPTION_TAG;
    d[25] = track_volume_val;
    d
}

#[inline(always)]
pub fn encode_pumpfun_buy_exact_sol_in_ix_data(
    spendable_sol_in: u64,
    min_tokens_out: u64,
    track_volume_val: u8,
) -> [u8; 26] {
    let mut d = [0u8; 26];
    d[..8].copy_from_slice(&BUY_EXACT_SOL_IN_DISCRIMINATOR);
    d[8..16].copy_from_slice(&spendable_sol_in.to_le_bytes());
    d[16..24].copy_from_slice(&min_tokens_out.to_le_bytes());
    d[24] = TRACK_VOLUME_OPTION_TAG;
    d[25] = track_volume_val;
    d
}

#[inline(always)]
pub fn encode_pumpfun_sell_ix_data(token_amount: u64, min_sol_output: u64) -> [u8; 24] {
    let mut d = [0u8; 24];
    d[..8].copy_from_slice(&SELL_DISCRIMINATOR);
    d[8..16].copy_from_slice(&token_amount.to_le_bytes());
    d[16..24].copy_from_slice(&min_sol_output.to_le_bytes());
    d
}

// --- v2 instruction data encoders (no track_volume arg — 2 args each, 24 bytes total) ---

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
pub fn encode_pumpfun_sell_v2_ix_data(amount: u64, min_sol_output: u64) -> [u8; 24] {
    let mut d = [0u8; 24];
    d[..8].copy_from_slice(&SELL_V2_DISCRIMINATOR);
    d[8..16].copy_from_slice(&amount.to_le_bytes());
    d[16..24].copy_from_slice(&min_sol_output.to_le_bytes());
    d
}
