use crate::*;
use core::result::Result::Ok;
use solana_sdk::{account::Account, clock::Clock};
use std::collections::HashMap;
use anyhow::{ensure, Context};
use solana_streamer::streaming::event_parser::protocols::meteora_dlmm::types::LbPair;
use crate::instruction::utils::meteora_dlmm::extensions::{ActivationType, Bin, BinArray, BinArrayBitmapExtExtension, BinArrayBitmapExtension, BinArrayExtension, BinExtension, LbPairExtension, PairStatus, PairType};
use crate::instruction::utils::meteora_dlmm::pda::derive_bin_array_pda;
use crate::instruction::utils::meteora_dlmm::typedefs::SwapResult;
use anyhow::Result;

#[derive(Debug)]
pub struct SwapExactInQuote {
    pub amount_out: u64,
    pub fee: u64,
}

#[derive(Debug)]
pub struct SwapExactOutQuote {
    pub amount_in: u64,
    pub fee: u64,
}

fn validate_swap_activation(
    lb_pair: &LbPair,
    current_timestamp: u64,
    current_slot: u64,
) -> Result<()> {
    ensure!(
        lb_pair.status()?.eq(&PairStatus::Enabled),
        "Pair is disabled"
    );

    let pair_type = lb_pair.pair_type()?;
    if pair_type.eq(&PairType::Permission) {
        let activation_type = lb_pair.activation_type()?;
        let current_point = match activation_type {
            ActivationType::Slot => current_slot,
            ActivationType::Timestamp => current_timestamp,
        };

        ensure!(
            current_point >= lb_pair.activation_point,
            "Pair is disabled"
        );
    }

    Ok(())
}

fn shift_active_bin_if_empty_gap(
    lb_pair: &mut LbPair,
    active_bin_array: &BinArray,
    swap_for_y: bool,
) -> Result<()> {
    let lb_pair_bin_array_index = BinArray::bin_id_to_bin_array_index(lb_pair.active_id)?;

    if i64::from(lb_pair_bin_array_index) != active_bin_array.index {
        if swap_for_y {
            let (_, upper_bin_id) =
                BinArray::get_bin_array_lower_upper_bin_id(active_bin_array.index as i32)?;
            lb_pair.active_id = upper_bin_id;
        } else {
            let (lower_bin_id, _) =
                BinArray::get_bin_array_lower_upper_bin_id(active_bin_array.index as i32)?;
            lb_pair.active_id = lower_bin_id;
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn quote_exact_out(
    lb_pair_pubkey: Pubkey,
    lb_pair: &LbPair,
    mut amount_out: u64,
    swap_for_y: bool,
    bin_arrays: HashMap<Pubkey, BinArray>,
    bitmap_extension: Option<&BinArrayBitmapExtension>,
    clock: &Clock,
    mint_x_account: &Account,
    mint_y_account: &Account,
) -> Result<SwapExactOutQuote> {
    let current_timestamp = clock.unix_timestamp as u64;
    let current_slot = clock.slot;
    let epoch = clock.epoch;

    validate_swap_activation(lb_pair, current_timestamp, current_slot)?;

    let mut lb_pair = *lb_pair;
    lb_pair.update_references(current_timestamp as i64)?;

    let mut total_amount_in: u64 = 0;
    let mut total_fee: u64 = 0;

    let (in_mint_account, out_mint_account) = if swap_for_y {
        (mint_x_account, mint_y_account)
    } else {
        (mint_y_account, mint_x_account)
    };

    while amount_out > 0 {
        let active_bin_array_pubkey = get_bin_array_pubkeys_for_swap(
            lb_pair_pubkey,
            &lb_pair,
            bitmap_extension,
            swap_for_y,
            1,
        )?
            .pop()
            .context("Pool out of liquidity")?;

        let mut active_bin_array = bin_arrays
            .get(&active_bin_array_pubkey)
            .cloned()
            .context("Active bin array not found")?;

        shift_active_bin_if_empty_gap(&mut lb_pair, &active_bin_array, swap_for_y)?;

        loop {
            if !active_bin_array.is_bin_id_within_range(lb_pair.active_id)? || amount_out == 0 {
                break;
            }

            lb_pair.update_volatility_accumulator()?;

            let active_bin = active_bin_array.get_bin_mut(lb_pair.active_id)?;
            let price = active_bin.get_or_store_bin_price(lb_pair.active_id, lb_pair.bin_step)?;

            if !active_bin.is_empty(!swap_for_y) {
                let bin_max_amount_out = active_bin.get_max_amount_out(swap_for_y);
                if amount_out >= bin_max_amount_out {
                    let max_amount_in = active_bin.get_max_amount_in(price, swap_for_y)?;
                    let max_fee = lb_pair.compute_fee(max_amount_in)?;

                    total_amount_in = total_amount_in
                        .checked_add(max_amount_in)
                        .context("MathOverflow")?;

                    total_fee = total_fee.checked_add(max_fee).context("MathOverflow")?;

                    amount_out = amount_out
                        .checked_sub(bin_max_amount_out)
                        .context("MathOverflow")?;
                } else {
                    let amount_in = Bin::get_amount_in(amount_out, price, swap_for_y)?;
                    let fee = lb_pair.compute_fee(amount_in)?;

                    total_amount_in = total_amount_in
                        .checked_add(amount_in)
                        .context("MathOverflow")?;

                    total_fee = total_fee.checked_add(fee).context("MathOverflow")?;

                    amount_out = 0;
                }
            }

            if amount_out > 0 {
                lb_pair.advance_active_bin(swap_for_y)?;
            }
        }
    }

    total_amount_in = total_amount_in
        .checked_add(total_fee)
        .context("MathOverflow")?;

    Ok(SwapExactOutQuote {
        amount_in: total_amount_in,
        fee: total_fee,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn quote_exact_in(
    lb_pair_pubkey: Pubkey,
    lb_pair: &LbPair,
    amount_in: u64,
    swap_for_y: bool,
    bin_arrays: HashMap<Pubkey, BinArray>,
    bitmap_extension: Option<&BinArrayBitmapExtension>,
    clock: &Clock,
    mint_x_account: &Account,
    mint_y_account: &Account,
) -> Result<SwapExactInQuote> {
    let current_timestamp = clock.unix_timestamp as u64;
    let current_slot = clock.slot;
    let epoch = clock.epoch;

    validate_swap_activation(lb_pair, current_timestamp, current_slot)?;

    let mut lb_pair = *lb_pair;
    lb_pair.update_references(current_timestamp as i64)?;

    let mut total_amount_out: u64 = 0;
    let mut total_fee: u64 = 0;

    let (in_mint_account, out_mint_account) = if swap_for_y {
        (mint_x_account, mint_y_account)
    } else {
        (mint_y_account, mint_x_account)
    };

    let mut amount_left = amount_in;

    while amount_left > 0 {
        let active_bin_array_pubkey = get_bin_array_pubkeys_for_swap(
            lb_pair_pubkey,
            &lb_pair,
            bitmap_extension,
            swap_for_y,
            1,
        )?
            .pop()
            .context("Pool out of liquidity")?;

        let mut active_bin_array = bin_arrays
            .get(&active_bin_array_pubkey)
            .cloned()
            .context("Active bin array not found")?;

        shift_active_bin_if_empty_gap(&mut lb_pair, &active_bin_array, swap_for_y)?;

        loop {
            if !active_bin_array.is_bin_id_within_range(lb_pair.active_id)? || amount_left == 0 {
                break;
            }

            lb_pair.update_volatility_accumulator()?;

            let active_bin = active_bin_array.get_bin_mut(lb_pair.active_id)?;
            let price = active_bin.get_or_store_bin_price(lb_pair.active_id, lb_pair.bin_step)?;

            if !active_bin.is_empty(!swap_for_y) {
                let SwapResult {
                    amount_in_with_fees,
                    amount_out,
                    fee,
                    ..
                } = active_bin.swap(amount_left, price, swap_for_y, &lb_pair, None)?;

                amount_left = amount_left
                    .checked_sub(amount_in_with_fees)
                    .context("MathOverflow")?;

                total_amount_out = total_amount_out
                    .checked_add(amount_out)
                    .context("MathOverflow")?;
                total_fee = total_fee.checked_add(fee).context("MathOverflow")?;
            }

            if amount_left > 0 {
                lb_pair.advance_active_bin(swap_for_y)?;
            }
        }
    }

    Ok(SwapExactInQuote {
        amount_out: total_amount_out,
        fee: total_fee,
    })
}

pub fn get_bin_array_pubkeys_for_swap(
    lb_pair_pubkey: Pubkey,
    lb_pair: &LbPair,
    bitmap_extension: Option<&BinArrayBitmapExtension>,
    swap_for_y: bool,
    take_count: u8,
) -> Result<Vec<Pubkey>> {
    let mut start_bin_array_idx = BinArray::bin_id_to_bin_array_index(lb_pair.active_id)?;
    let mut bin_array_idx = vec![];
    let increment = if swap_for_y { -1 } else { 1 };

    loop {
        if bin_array_idx.len() == take_count as usize {
            break;
        }

        if lb_pair.is_overflow_default_bin_array_bitmap(start_bin_array_idx) {
            let Some(bitmap_extension) = bitmap_extension else {
                break;
            };
            let Ok((next_bin_array_idx, has_liquidity)) = bitmap_extension
                .next_bin_array_index_with_liquidity(swap_for_y, start_bin_array_idx)
            else {
                // Out of search range. No liquidity.
                break;
            };
            if has_liquidity {
                bin_array_idx.push(next_bin_array_idx);
                start_bin_array_idx = next_bin_array_idx + increment;
            } else {
                // Switch to internal bitmap
                start_bin_array_idx = next_bin_array_idx;
            }
        } else {
            let Ok((next_bin_array_idx, has_liquidity)) = lb_pair
                .next_bin_array_index_with_liquidity_internal(swap_for_y, start_bin_array_idx)
            else {
                break;
            };
            if has_liquidity {
                bin_array_idx.push(next_bin_array_idx);
                start_bin_array_idx = next_bin_array_idx + increment;
            } else {
                // Switch to external bitmap
                start_bin_array_idx = next_bin_array_idx;
            }
        }
    }

    let bin_array_pubkeys = bin_array_idx
        .into_iter()
        .map(|idx| derive_bin_array_pda(lb_pair_pubkey, idx.into()).0)
        .collect();

    Ok(bin_array_pubkeys)
}
