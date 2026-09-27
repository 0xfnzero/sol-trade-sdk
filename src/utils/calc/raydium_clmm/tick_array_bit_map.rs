// Vendored from https://github.com/raydium-io/raydium-clmm at ed7c84a54ced59c55981780546adb0b4583dcf85
// (programs/amm/src/libraries/tick_array_bit_map.rs), Apache-2.0: see LICENSE in this directory.
// Changes: tests removed; formatted with this repository's rustfmt; Anchor errors
// replaced by the local `ErrorCode`; shift amounts converted to usize explicitly
// for the `uint` crate's U1024.

///! Helper functions to get most and least significant non-zero bits
use super::big_num::U1024;
use super::error::ErrorCode;
use super::error::Result;
use super::state::{TickArrayState, TickState, TICK_ARRAY_SIZE};

pub const TICK_ARRAY_BITMAP_SIZE: i32 = 512;

pub type TickArryBitmap = [u64; 8];

pub fn max_tick_in_tickarray_bitmap(tick_spacing: u16) -> i32 {
    i32::from(tick_spacing) * TICK_ARRAY_SIZE * TICK_ARRAY_BITMAP_SIZE
}

pub fn get_bitmap_tick_boundary(tick_array_start_index: i32, tick_spacing: u16) -> (i32, i32) {
    let ticks_in_one_bitmap: i32 = max_tick_in_tickarray_bitmap(tick_spacing);
    let mut m = tick_array_start_index.abs() / ticks_in_one_bitmap;
    if tick_array_start_index < 0 && tick_array_start_index.abs() % ticks_in_one_bitmap != 0 {
        m += 1;
    }
    let min_value: i32 = ticks_in_one_bitmap * m;
    if tick_array_start_index < 0 {
        (-min_value, -min_value + ticks_in_one_bitmap)
    } else {
        (min_value, min_value + ticks_in_one_bitmap)
    }
}

pub fn most_significant_bit(x: U1024) -> Option<u16> {
    if x.is_zero() {
        None
    } else {
        Some(u16::try_from(x.leading_zeros()).unwrap())
    }
}

pub fn least_significant_bit(x: U1024) -> Option<u16> {
    if x.is_zero() {
        None
    } else {
        Some(u16::try_from(x.trailing_zeros()).unwrap())
    }
}

/// Given a tick, calculate whether the tickarray it belongs to has been initialized.
/// Note: The caller of the function should ensure that tick_current is within the range represented by bit_map.
/// Currently, this function is only called when `bit_map = pool.tick_array_bitmap`.
pub fn check_current_tick_array_is_initialized(
    bit_map: U1024,
    tick_current: i32,
    tick_spacing: u16,
) -> Result<(bool, i32)> {
    if TickState::check_is_out_of_boundary(tick_current) {
        return err!(ErrorCode::InvalidTickIndex);
    }
    let multiplier = i32::from(tick_spacing) * TICK_ARRAY_SIZE;
    let mut compressed = tick_current / multiplier + 512;
    if tick_current < 0 && tick_current % multiplier != 0 {
        // round towards negative infinity
        compressed -= 1;
    }
    let bit_pos = compressed.abs();
    // set current bit
    let mask = U1024::one() << usize::try_from(bit_pos).unwrap();
    let masked = bit_map & mask;
    // check the current bit whether initialized
    let initialized = masked != U1024::default();
    if initialized {
        return Ok((true, (compressed - 512) * multiplier));
    }
    // the current bit is not initialized
    return Ok((false, (compressed - 512) * multiplier));
}

/// The function is only called when `bit_map = pool.tick_array_bitmap`.
pub fn next_initialized_tick_array_start_index(
    bit_map: U1024,
    last_tick_array_start_index: i32,
    tick_spacing: u16,
    zero_for_one: bool,
) -> Result<(bool, i32)> {
    require!(
        TickArrayState::check_is_valid_start_index(last_tick_array_start_index, tick_spacing),
        ErrorCode::InvalidTickArrayBoundary
    );
    let tick_boundary = max_tick_in_tickarray_bitmap(tick_spacing);
    let next_tick_array_start_index = if zero_for_one {
        last_tick_array_start_index - TickArrayState::tick_count(tick_spacing)
    } else {
        last_tick_array_start_index + TickArrayState::tick_count(tick_spacing)
    };

    if next_tick_array_start_index < -tick_boundary || next_tick_array_start_index >= tick_boundary
    {
        return Ok((false, last_tick_array_start_index));
    }

    let multiplier = i32::from(tick_spacing) * TICK_ARRAY_SIZE;
    let mut compressed = next_tick_array_start_index / multiplier + 512;
    if next_tick_array_start_index < 0 && next_tick_array_start_index % multiplier != 0 {
        // round towards negative infinity
        compressed -= 1;
    }
    let bit_pos = compressed.abs();
    Ok(if zero_for_one {
        // tick from upper to lower
        // find from highter bits to lower bits
        let offset_bit_map = bit_map << usize::try_from(1024 - bit_pos - 1).unwrap();
        if let Some(next_bit) = most_significant_bit(offset_bit_map) {
            let next_array_start_index = (bit_pos - i32::from(next_bit) - 512) * multiplier;
            (true, next_array_start_index)
        } else {
            // not found til to the end
            (false, -tick_boundary)
        }
    } else {
        // tick from lower to upper
        // find from lower bits to highter bits
        let offset_bit_map = bit_map >> usize::try_from(bit_pos).unwrap();
        if let Some(next_bit) = least_significant_bit(offset_bit_map) {
            let next_array_start_index = (bit_pos + i32::from(next_bit) - 512) * multiplier;
            (true, next_array_start_index)
        } else {
            // not found til to the end
            (false, tick_boundary - TickArrayState::tick_count(tick_spacing))
        }
    })
}
