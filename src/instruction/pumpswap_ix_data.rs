//! PumpSwap AMM `buy` / `buy_exact_quote_in` / `sell` instruction **data**（栈数组、无 `Vec` 分配）。

use crate::instruction::utils::pumpswap::{
    BUY_DISCRIMINATOR, BUY_EXACT_QUOTE_IN_DISCRIMINATOR, SELL_DISCRIMINATOR,
};

#[inline(always)]
pub fn encode_pumpswap_buy_ix_data(
    base_amount_out: u64,
    max_quote_amount_in: u64,
    track_volume: u8,
) -> [u8; 25] {
    let mut d = [0u8; 25];
    d[..8].copy_from_slice(&BUY_DISCRIMINATOR);
    d[8..16].copy_from_slice(&base_amount_out.to_le_bytes());
    d[16..24].copy_from_slice(&max_quote_amount_in.to_le_bytes());
    d[24] = track_volume;
    d
}

#[inline(always)]
pub fn encode_pumpswap_buy_exact_quote_in_ix_data(
    spendable_quote_in: u64,
    min_base_amount_out: u64,
    track_volume: u8,
) -> [u8; 25] {
    let mut d = [0u8; 25];
    d[..8].copy_from_slice(&BUY_EXACT_QUOTE_IN_DISCRIMINATOR);
    d[8..16].copy_from_slice(&spendable_quote_in.to_le_bytes());
    d[16..24].copy_from_slice(&min_base_amount_out.to_le_bytes());
    d[24] = track_volume;
    d
}

#[inline(always)]
pub fn encode_pumpswap_sell_ix_data(base_amount_in: u64, min_quote_amount_out: u64) -> [u8; 24] {
    let mut d = [0u8; 24];
    d[..8].copy_from_slice(&SELL_DISCRIMINATOR);
    d[8..16].copy_from_slice(&base_amount_in.to_le_bytes());
    d[16..24].copy_from_slice(&min_quote_amount_out.to_le_bytes());
    d
}
