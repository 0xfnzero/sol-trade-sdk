// Ported from https://github.com/raydium-io/raydium-clmm at ed7c84a54ced59c55981780546adb0b4583dcf85
// (programs/amm/src/states/{tick_array,pool,pool_fee,tickarray_bitmap_extension}.rs),
// Apache-2.0: see LICENSE in this directory.
// Changes: accounts decode from raw bytes into plain structs holding only what a
// swap reads; state writes a swap makes are left out; Anchor errors replaced by
// the local `ErrorCode`.

//! Accounts a swap reads, with the program's navigation over them.

// Bound checks are kept as the program writes them.
#![allow(clippy::manual_range_contains)]

use super::big_num::{U1024, U128, U512};
use super::config::FEE_RATE_DENOMINATOR_VALUE;
use super::error::{ErrorCode, Result};
use super::full_math::MulDiv;
use super::tick_array_bit_map::{
    check_current_tick_array_is_initialized, get_bitmap_tick_boundary,
    max_tick_in_tickarray_bitmap, next_initialized_tick_array_start_index as default_next,
    TickArryBitmap, TICK_ARRAY_BITMAP_SIZE,
};
use super::{fixed_point_64, tick_math};
use solana_sdk::pubkey::Pubkey;

pub const TICK_ARRAY_SIZE_USIZE: usize = 60;
pub const TICK_ARRAY_SIZE: i32 = 60;

pub const POOL_STATE_DISCRIMINATOR: [u8; 8] = [247, 237, 227, 245, 215, 195, 222, 70];
pub const TICK_ARRAY_STATE_DISCRIMINATOR: [u8; 8] = [192, 155, 85, 205, 49, 249, 129, 42];
pub const TICK_ARRAY_BITMAP_EXTENSION_DISCRIMINATOR: [u8; 8] =
    [60, 150, 36, 219, 97, 128, 139, 153];

// Maximum fee rate numerator, 10%
pub const MAX_FEE_RATE_NUMERATOR: u32 = 100_000;
pub const VOLATILITY_ACCUMULATOR_SCALE: u16 = 10_000;
pub const REDUCTION_FACTOR_DENOMINATOR: u16 = 10_000;
pub const DYNAMIC_FEE_CONTROL_DENOMINATOR: u32 = 100_000;

const EXTENSION_TICKARRAY_BITMAP_SIZE: usize = 14;

fn pubkey_at(data: &[u8], offset: usize) -> Pubkey {
    Pubkey::new_from_array(data[offset..offset + 32].try_into().unwrap())
}

fn u16_at(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(data[offset..offset + 2].try_into().unwrap())
}

fn u32_at(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn i32_at(data: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn u64_at(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

fn u128_at(data: &[u8], offset: usize) -> u128 {
    u128::from_le_bytes(data[offset..offset + 16].try_into().unwrap())
}

fn i128_at(data: &[u8], offset: usize) -> i128 {
    i128::from_le_bytes(data[offset..offset + 16].try_into().unwrap())
}

/// Result of limit order matching
#[derive(Debug, Clone, Copy, Default)]
pub struct LimitOrderMatchResult {
    /// Amount of input tokens consumed by the limit order
    pub amount_in: u64,
    /// Amount of output tokens produced by the limit order
    pub amount_out: u64,
    /// Amount of fee tokens paid by the swap taker
    pub amm_fee_amount: u64,
}

/// A tick, without the fee and reward growth bookkeeping a quote does not need.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TickState {
    pub tick: i32,
    /// Amount of net liquidity added (subtracted) when tick is crossed from left to right (right to left)
    pub liquidity_net: i128,
    /// The total position liquidity that references this tick
    pub liquidity_gross: u128,
    /// Order phase of the tick, used as a FIFO cohort index for limit orders
    pub order_phase: u64,
    /// The amount of limit orders that have never been matched
    pub orders_amount: u64,
    /// Remaining part filled orders amount
    pub part_filled_orders_remaining: u64,
    /// Cumulative unfilled ratio for the current part-filled cohort (Q64.64 format).
    pub unfilled_ratio_x64: u128,
}

impl TickState {
    pub const LEN: usize = 168;

    /// Packed layout: tick @0, liquidity_net @4, liquidity_gross @20, fee and reward
    /// growths @36..116, order_phase @116, orders_amount @124,
    /// part_filled_orders_remaining @132, unfilled_ratio_x64 @140, padding.
    fn decode(data: &[u8]) -> Self {
        Self {
            tick: i32_at(data, 0),
            liquidity_net: i128_at(data, 4),
            liquidity_gross: u128_at(data, 20),
            order_phase: u64_at(data, 116),
            orders_amount: u64_at(data, 124),
            part_filled_orders_remaining: u64_at(data, 132),
            unfilled_ratio_x64: u128_at(data, 140),
        }
    }

    pub fn is_initialized(&self) -> bool {
        self.has_liquidity() || self.has_limit_orders()
    }

    pub fn has_limit_orders(&self) -> bool {
        self.orders_amount > 0 || self.part_filled_orders_remaining > 0
    }

    pub fn has_liquidity(&self) -> bool {
        self.liquidity_gross > 0
    }

    /// [`get_limit_order_output`] with a caller-provided `token_0_price_x64`.
    /// `token_0_price_x64` must equal `get_price_at_tick(tick, !zero_for_one)`.
    fn get_limit_order_output_with_price(
        amount_in: u64,
        token_0_price_x64: U128,
        zero_for_one: bool,
    ) -> Result<U128> {
        let output_amount = if zero_for_one {
            // token1_amount = token0_amount * token_0_price_x64 / 2^64
            U128::from(amount_in).mul_div_floor(token_0_price_x64, U128::from(fixed_point_64::Q64))
        } else {
            // token0_amount = token1_amount * 2^64 / token_0_price_x64
            U128::from(amount_in).mul_div_floor(U128::from(fixed_point_64::Q64), token_0_price_x64)
        }
        .ok_or(ErrorCode::CalculateOverflow)?;
        Ok(output_amount)
    }

    /// Given the output amount from a limit order, calculate the required input
    /// token amount (rounded up). `token_0_price_x64` must equal
    /// `get_price_at_tick(tick, zero_for_one)`.
    pub fn get_limit_order_input_with_price(
        amount_out: u64,
        token_0_price_x64: U128,
        zero_for_one: bool,
    ) -> Result<u64> {
        let amount_in = if zero_for_one {
            // token1_consumed = token0_executed * token_0_price_x64 / 2^64
            U128::from(amount_out)
                .mul_div_ceil(token_0_price_x64, U128::from(fixed_point_64::Q64))
                .ok_or(ErrorCode::CalculateOverflow)?
        } else {
            // token0_consumed = token1_executed * 2^64 / token_0_price_x64
            U128::from(amount_out)
                .mul_div_ceil(U128::from(fixed_point_64::Q64), token_0_price_x64)
                .ok_or(ErrorCode::CalculateOverflow)?
        };
        if amount_in > U128::from(u64::MAX) {
            return err!(ErrorCode::CalculateOverflow);
        }
        Ok(amount_in.as_u64())
    }

    pub fn limit_order_unfilled_amount(&self) -> Result<u64> {
        let total_unfilled_amount = self
            .orders_amount
            .checked_add(self.part_filled_orders_remaining)
            .ok_or(ErrorCode::CalculateOverflow)?;
        Ok(total_unfilled_amount)
    }

    pub fn match_limit_order_with_sqrt_price(
        &mut self,
        swap_amount: u64,
        swap_direction_zero_for_one: bool,
        is_base_input: bool,
        fee_rate: u32,
        is_fee_on_input: bool,
        sqrt_price_x64: u128,
    ) -> Result<LimitOrderMatchResult> {
        let mut result = LimitOrderMatchResult::default();

        let total_unfilled_amount = self.limit_order_unfilled_amount()?;
        if swap_amount == 0 || total_unfilled_amount == 0 {
            return Ok(result);
        }

        let token_0_price_x64 =
            tick_math::get_price_from_sqrt_price(sqrt_price_x64, !swap_direction_zero_for_one)?;

        if is_base_input {
            // Assume the input amount can be fully consumed, calculate the amount of limit order tokens matched
            if is_fee_on_input {
                result.amm_fee_amount = swap_amount
                    .mul_div_ceil(fee_rate.into(), u64::from(FEE_RATE_DENOMINATOR_VALUE))
                    .ok_or(ErrorCode::CalculateOverflow)?;
                result.amount_in = swap_amount - result.amm_fee_amount;
            } else {
                result.amount_in = swap_amount;
            }
            let matched_output = TickState::get_limit_order_output_with_price(
                result.amount_in,
                token_0_price_x64,
                swap_direction_zero_for_one,
            )?;
            // If the amount of limit order tokens matched is greater than the total unfilled amount,
            // it means the input cannot be fully consumed, so recalculate the input and output amounts
            if matched_output > U128::from(total_unfilled_amount) {
                result.amount_out = total_unfilled_amount;
                result.amount_in = TickState::get_limit_order_input_with_price(
                    total_unfilled_amount,
                    token_0_price_x64,
                    !swap_direction_zero_for_one,
                )?;
                if is_fee_on_input {
                    result.amm_fee_amount = result
                        .amount_in
                        .mul_div_ceil(
                            fee_rate.into(),
                            u64::from(FEE_RATE_DENOMINATOR_VALUE - fee_rate),
                        )
                        .ok_or(ErrorCode::CalculateOverflow)?;
                }
                // Fee from output will be calculated at the end
            } else {
                result.amount_out = matched_output.as_u64();
            }
        } else {
            // swap_amount is the desired net output (after fee deduction if fee is from output)
            let net_output = swap_amount.min(total_unfilled_amount);
            result.amount_out = if is_fee_on_input {
                net_output
            } else {
                // total_output = net_output / (1 - fee_rate / FEE_RATE_DENOMINATOR)
                net_output
                    .mul_div_ceil(
                        u64::from(FEE_RATE_DENOMINATOR_VALUE),
                        (FEE_RATE_DENOMINATOR_VALUE - fee_rate).into(),
                    )
                    .ok_or(ErrorCode::CalculateOverflow)?
                    .min(total_unfilled_amount)
            };
            result.amount_in = TickState::get_limit_order_input_with_price(
                result.amount_out,
                token_0_price_x64,
                !swap_direction_zero_for_one,
            )?;
            if is_fee_on_input {
                result.amm_fee_amount = result
                    .amount_in
                    .mul_div_ceil(fee_rate.into(), u64::from(FEE_RATE_DENOMINATOR_VALUE - fee_rate))
                    .ok_or(ErrorCode::CalculateOverflow)?;
            }
            // Fee from output will be calculated at the end
        }

        let mut consume_from_part_remaining = 0;
        // Consume part_filled_orders_remaining first (FIFO priority)
        if self.part_filled_orders_remaining > 0 {
            consume_from_part_remaining = self.part_filled_orders_remaining.min(result.amount_out);
            // Update unfilled_ratio: ratio *= (remaining - consumed) / remaining
            if consume_from_part_remaining > 0 {
                self.unfilled_ratio_x64 = U128::from(self.unfilled_ratio_x64)
                    .mul_div_floor(
                        U128::from(self.part_filled_orders_remaining - consume_from_part_remaining),
                        U128::from(self.part_filled_orders_remaining),
                    )
                    .ok_or(ErrorCode::CalculateOverflow)?
                    .as_u128();
            }
            self.part_filled_orders_remaining =
                self.part_filled_orders_remaining.saturating_sub(consume_from_part_remaining);
        }
        let amount_out_continue_to_consume =
            result.amount_out.saturating_sub(consume_from_part_remaining);

        // If there is still more to consume, consume from orders_amount
        if amount_out_continue_to_consume > 0 {
            require_eq!(self.part_filled_orders_remaining, 0);
            require_gte!(
                self.orders_amount,
                amount_out_continue_to_consume,
                ErrorCode::InvalidLimitOrderAmount
            );
            // Order phase increases when consuming from orders_amount
            self.order_phase = self.order_phase.saturating_add(1);

            // Reset unfilled_ratio for new phase, then update for consumption
            self.unfilled_ratio_x64 = U128::from(fixed_point_64::Q64)
                .mul_div_floor(
                    U128::from(self.orders_amount - amount_out_continue_to_consume),
                    U128::from(self.orders_amount),
                )
                .ok_or(ErrorCode::CalculateOverflow)?
                .as_u128();

            // Move remaining orders_amount to part_filled_orders_remaining
            self.part_filled_orders_remaining = self.orders_amount - amount_out_continue_to_consume;
            self.orders_amount = 0;
        }
        // Calculate fee and deduct from output if fee is from output (after limit order consumption calculation)
        if !is_fee_on_input {
            result.amm_fee_amount = result
                .amount_out
                .mul_div_ceil(fee_rate.into(), u64::from(FEE_RATE_DENOMINATOR_VALUE))
                .ok_or(ErrorCode::CalculateOverflow)?;
            // Deduct fee from output: user receives net output
            result.amount_out = result
                .amount_out
                .checked_sub(result.amm_fee_amount)
                .ok_or(ErrorCode::CalculateOverflow)?;
        }
        Ok(result)
    }

    /// Common checks for a valid tick input.
    /// A tick is valid if it lies within tick boundaries
    pub fn check_is_out_of_boundary(tick: i32) -> bool {
        tick < tick_math::MIN_TICK || tick > tick_math::MAX_TICK
    }
}

/// A tick array account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickArrayState {
    pub pool_id: Pubkey,
    pub start_tick_index: i32,
    pub ticks: Vec<TickState>,
    pub initialized_tick_count: u8,
}

impl TickArrayState {
    pub const LEN: usize = 8 + 32 + 4 + TickState::LEN * TICK_ARRAY_SIZE_USIZE + 1 + 115;

    /// Layout: discriminator, pool_id @8, start_tick_index @40, 60 ticks of 168
    /// bytes @44, initialized_tick_count @10124, recent_epoch, padding.
    pub fn decode(data: &[u8]) -> Result<Self> {
        if data.len() != Self::LEN || data[..8] != TICK_ARRAY_STATE_DISCRIMINATOR {
            return Err(ErrorCode::InvalidAccountData);
        }
        let ticks = (0..TICK_ARRAY_SIZE_USIZE)
            .map(|i| TickState::decode(&data[44 + i * TickState::LEN..]))
            .collect();
        Ok(Self {
            pool_id: pubkey_at(data, 8),
            start_tick_index: i32_at(data, 40),
            ticks,
            initialized_tick_count: data[44 + TickState::LEN * TICK_ARRAY_SIZE_USIZE],
        })
    }

    /// Base on swap direction, the index of the first initialized tick in the tick array.
    pub fn first_initialized_tick(&self, zero_for_one: bool) -> Result<usize> {
        if zero_for_one {
            (0..TICK_ARRAY_SIZE_USIZE).rev().find(|&i| self.ticks[i].is_initialized())
        } else {
            (0..TICK_ARRAY_SIZE_USIZE).find(|&i| self.ticks[i].is_initialized())
        }
        .ok_or(ErrorCode::InvalidTickArray)
    }

    /// Index of the next initialized tick in the tick array: price moving to the
    /// left tick <= current_tick_index, or to the right tick > current_tick_index.
    pub fn next_initialized_tick(
        &self,
        current_tick_index: i32,
        tick_spacing: u16,
        zero_for_one: bool,
    ) -> Option<usize> {
        let current_tick_array_start_index =
            TickArrayState::get_array_start_index(current_tick_index, tick_spacing);
        if current_tick_array_start_index != self.start_tick_index {
            return None;
        }
        let offset_in_array =
            (current_tick_index - self.start_tick_index) / i32::from(tick_spacing);

        let found_index = if zero_for_one {
            (0..=offset_in_array).rev().find(|&i| self.ticks[i as usize].is_initialized())
        } else {
            ((offset_in_array + 1)..TICK_ARRAY_SIZE)
                .find(|&i| self.ticks[i as usize].is_initialized())
        };
        found_index.map(|i| i as usize)
    }

    /// Input an arbitrary tick_index, output the start_index of the tick_array it sits on
    pub fn get_array_start_index(tick_index: i32, tick_spacing: u16) -> i32 {
        let ticks_in_array = TickArrayState::tick_count(tick_spacing);
        let mut start = tick_index / ticks_in_array;
        if tick_index < 0 && tick_index % ticks_in_array != 0 {
            start -= 1
        }
        start * ticks_in_array
    }

    pub fn check_is_valid_start_index(tick_index: i32, tick_spacing: u16) -> bool {
        if TickState::check_is_out_of_boundary(tick_index) {
            if tick_index > tick_math::MAX_TICK {
                return false;
            }
            let min_start_index =
                TickArrayState::get_array_start_index(tick_math::MIN_TICK, tick_spacing);
            return tick_index == min_start_index;
        }
        tick_index % TickArrayState::tick_count(tick_spacing) == 0
    }

    pub fn tick_count(tick_spacing: u16) -> i32 {
        TICK_ARRAY_SIZE * i32::from(tick_spacing)
    }
}

/// Dynamic fee information for pool configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DynamicFeeInfo {
    /// Period that determines the high frequency trading time window (in seconds).
    pub filter_period: u16,
    /// Period that determines when the dynamic fee starts to decrease (in seconds).
    pub decay_period: u16,
    /// Dynamic fee rate decrement rate, used for volatility reference decay.
    pub reduction_factor: u16,
    /// Factor used to scale the dynamic fee component in the fee rate calculation.
    pub dynamic_fee_control: u32,
    /// Maximum value for the volatility accumulator, used to cap the dynamic fee rate.
    pub max_volatility_accumulator: u32,
    /// Active tick spacing index at the last reference update.
    pub tick_spacing_index_reference: i32,
    /// Volatility reference value, stores the decayed volatility accumulator.
    pub volatility_reference: u32,
    /// Volatility accumulator, used to calculate the dynamic fee rate.
    pub volatility_accumulator: u32,
    /// Last timestamp (block time) when the references were updated.
    pub last_update_timestamp: u64,
    /// Reserved for future upgrades; part of the "is enabled" comparison.
    pub padding: [u8; 46],
}

impl Default for DynamicFeeInfo {
    fn default() -> Self {
        Self {
            filter_period: 0,
            decay_period: 0,
            reduction_factor: 0,
            dynamic_fee_control: 0,
            max_volatility_accumulator: 0,
            last_update_timestamp: 0,
            volatility_reference: 0,
            tick_spacing_index_reference: 0,
            volatility_accumulator: 0,
            padding: [0u8; 46],
        }
    }
}

impl DynamicFeeInfo {
    pub const LEN: usize = 80;

    fn decode(data: &[u8]) -> Self {
        Self {
            filter_period: u16_at(data, 0),
            decay_period: u16_at(data, 2),
            reduction_factor: u16_at(data, 4),
            dynamic_fee_control: u32_at(data, 6),
            max_volatility_accumulator: u32_at(data, 10),
            tick_spacing_index_reference: i32_at(data, 14),
            volatility_reference: u32_at(data, 18),
            volatility_accumulator: u32_at(data, 22),
            last_update_timestamp: u64_at(data, 26),
            padding: data[34..80].try_into().unwrap(),
        }
    }

    /// Updates the volatility accumulator based on the distance from the reference tick spacing index.
    pub fn update_volatility_accumulator(&mut self, tick_spacing_index: i32) -> Result<()> {
        // Calculate the absolute distance in tick groups from the reference point
        let index_delta = (self.tick_spacing_index_reference - tick_spacing_index).unsigned_abs();
        let volatility_accumulator = u64::from(self.volatility_reference)
            + u64::from(index_delta) * u64::from(VOLATILITY_ACCUMULATOR_SCALE);

        // Clamp to maximum value to prevent excessive fee rates
        self.volatility_accumulator =
            std::cmp::min(volatility_accumulator, u64::from(self.max_volatility_accumulator))
                as u32;

        Ok(())
    }

    /// Updates the volatility reference and tick spacing index reference based on time windows.
    pub fn update_reference(
        &mut self,
        tick_spacing_index: i32,
        current_timestamp: u64,
    ) -> Result<()> {
        let time_since_reference_update =
            current_timestamp.saturating_sub(self.last_update_timestamp);

        if time_since_reference_update < self.filter_period as u64 {
            // High frequency trading period: no update to prevent excessive fee changes
        } else if time_since_reference_update < self.decay_period as u64 {
            // Decay period: update references with decayed volatility
            self.tick_spacing_index_reference = tick_spacing_index;
            self.volatility_reference =
                (u64::from(self.volatility_accumulator) * u64::from(self.reduction_factor)
                    / u64::from(REDUCTION_FACTOR_DENOMINATOR)) as u32;
            self.last_update_timestamp = current_timestamp;
        } else {
            // Out of decay time window: reset volatility reference to 0
            self.tick_spacing_index_reference = tick_spacing_index;
            self.volatility_reference = 0;
            self.last_update_timestamp = current_timestamp;
        }

        Ok(())
    }
}

pub fn tick_spacing_index_from_tick(tick_index: i32, tick_spacing: u16) -> i32 {
    let tick_spacing = i32::from(tick_spacing);
    if tick_index % tick_spacing == 0 || tick_index >= 0 {
        tick_index / tick_spacing
    } else {
        tick_index / tick_spacing - 1
    }
}

pub enum PoolStatusBitIndex {
    OpenPositionOrIncreaseLiquidity,
    DecreaseLiquidity,
    CollectFee,
    CollectReward,
    Swap,
    LimitOrder,
}

/// The pool fields a swap reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolState {
    pub amm_config: Pubkey,
    pub token_mint_0: Pubkey,
    pub token_mint_1: Pubkey,
    pub token_vault_0: Pubkey,
    pub token_vault_1: Pubkey,
    pub observation_key: Pubkey,
    pub tick_spacing: u16,
    pub liquidity: u128,
    pub sqrt_price_x64: u128,
    pub tick_current: i32,
    pub status: u8,
    /// Fee on which token (0 = FromInput, 1 = Token0Only, 2 = Token1Only)
    pub fee_on: u8,
    pub tick_array_bitmap: [u64; 16],
    pub open_time: u64,
    pub dynamic_fee_info: DynamicFeeInfo,
}

impl PoolState {
    pub const LEN: usize = 1544;

    /// Packed layout: discriminator, bump, amm_config @9, owner, token_mint_0 @73,
    /// token_mint_1 @105, token_vault_0 @137, token_vault_1 @169, observation_key
    /// @201, decimals, tick_spacing @235, liquidity @237, sqrt_price_x64 @253,
    /// tick_current @269, ..., status @389, fee_on @390, ..., reward_infos @397,
    /// tick_array_bitmap @904, ..., open_time @1080, recent_epoch,
    /// dynamic_fee_info @1096, padding.
    pub fn decode(data: &[u8]) -> Result<Self> {
        if data.len() != Self::LEN || data[..8] != POOL_STATE_DISCRIMINATOR {
            return Err(ErrorCode::InvalidAccountData);
        }
        let mut tick_array_bitmap = [0u64; 16];
        for (i, word) in tick_array_bitmap.iter_mut().enumerate() {
            *word = u64_at(data, 904 + 8 * i);
        }
        Ok(Self {
            amm_config: pubkey_at(data, 9),
            token_mint_0: pubkey_at(data, 73),
            token_mint_1: pubkey_at(data, 105),
            token_vault_0: pubkey_at(data, 137),
            token_vault_1: pubkey_at(data, 169),
            observation_key: pubkey_at(data, 201),
            tick_spacing: u16_at(data, 235),
            liquidity: u128_at(data, 237),
            sqrt_price_x64: u128_at(data, 253),
            tick_current: i32_at(data, 269),
            status: data[389],
            fee_on: data[390],
            tick_array_bitmap,
            open_time: u64_at(data, 1080),
            dynamic_fee_info: DynamicFeeInfo::decode(&data[1096..1176]),
        })
    }

    /// Get status by bit, if it is `normal` status, return true
    pub fn get_status_by_bit(&self, bit: PoolStatusBitIndex) -> bool {
        let status = 1u8 << (bit as u8);
        self.status & status == 0
    }

    pub fn get_dynamic_fee_info(&self) -> Option<DynamicFeeInfo> {
        if self.dynamic_fee_info == DynamicFeeInfo::default() {
            return None;
        }
        Some(self.dynamic_fee_info)
    }

    /// Determine if fee should be collected from input token
    pub fn is_fee_on_input(&self, zero_for_one: bool) -> bool {
        match self.fee_on {
            0 => true,
            1 => zero_for_one,
            2 => !zero_for_one,
            _ => true, // default to FromInput
        }
    }

    pub fn get_first_initialized_tick_array(
        &self,
        tickarray_bitmap_extension: Option<&TickArrayBitmapExtension>,
        zero_for_one: bool,
    ) -> Result<(bool, i32)> {
        let (is_initialized, start_index) =
            if self.is_overflow_default_tickarray_bitmap(&[self.tick_current]) {
                let extension = tickarray_bitmap_extension
                    .ok_or(ErrorCode::MissingTickArrayBitmapExtensionAccount)?;
                extension.check_tick_array_is_initialized(
                    TickArrayState::get_array_start_index(self.tick_current, self.tick_spacing),
                    self.tick_spacing,
                )?
            } else {
                check_current_tick_array_is_initialized(
                    U1024(self.tick_array_bitmap),
                    self.tick_current,
                    self.tick_spacing,
                )?
            };
        if is_initialized {
            return Ok((true, start_index));
        }
        let next_start_index = self
            .next_initialized_tick_array_start_index(
                tickarray_bitmap_extension,
                TickArrayState::get_array_start_index(self.tick_current, self.tick_spacing),
                zero_for_one,
            )?
            .ok_or(ErrorCode::InsufficientLiquidityForDirection)?;
        Ok((false, next_start_index))
    }

    pub fn next_initialized_tick_array_start_index(
        &self,
        tickarray_bitmap_extension: Option<&TickArrayBitmapExtension>,
        mut last_tick_array_start_index: i32,
        zero_for_one: bool,
    ) -> Result<Option<i32>> {
        last_tick_array_start_index =
            TickArrayState::get_array_start_index(last_tick_array_start_index, self.tick_spacing);

        loop {
            let (is_found, start_index) = default_next(
                U1024(self.tick_array_bitmap),
                last_tick_array_start_index,
                self.tick_spacing,
                zero_for_one,
            )?;
            if is_found {
                return Ok(Some(start_index));
            }
            // When tick_spacing >= 15 the default bitmap already spans the entire
            // [MIN_TICK, MAX_TICK] range, so a miss means the direction is exhausted.
            if self.tick_spacing >= 15 {
                return Ok(None);
            }
            last_tick_array_start_index = start_index;

            let extension = tickarray_bitmap_extension
                .ok_or(ErrorCode::MissingTickArrayBitmapExtensionAccount)?;

            let (is_found, start_index) = extension.next_initialized_tick_array_from_one_bitmap(
                last_tick_array_start_index,
                self.tick_spacing,
                zero_for_one,
            )?;
            if is_found {
                return Ok(Some(start_index));
            }
            last_tick_array_start_index = start_index;

            if last_tick_array_start_index < tick_math::MIN_TICK
                || last_tick_array_start_index > tick_math::MAX_TICK
            {
                return Ok(None);
            }
        }
    }

    pub fn is_overflow_default_tickarray_bitmap(&self, tick_indexs: &[i32]) -> bool {
        let (min_tick_array_start_index_boundary, max_tick_array_index_boundary) =
            self.tick_array_start_index_range();
        for &tick_index in tick_indexs {
            let tick_array_start_index =
                TickArrayState::get_array_start_index(tick_index, self.tick_spacing);
            if tick_array_start_index >= max_tick_array_index_boundary
                || tick_array_start_index < min_tick_array_start_index_boundary
            {
                return true;
            }
        }
        false
    }

    // the range of tick array start index that default tickarray bitmap can represent
    // if tick_spacing = 1, the result range is [-30720, 30720)
    pub fn tick_array_start_index_range(&self) -> (i32, i32) {
        // the range of ticks that default tickarray can represent
        let mut max_tick_boundary = max_tick_in_tickarray_bitmap(self.tick_spacing);
        let mut min_tick_boundary = -max_tick_boundary;
        if max_tick_boundary > tick_math::MAX_TICK {
            max_tick_boundary =
                TickArrayState::get_array_start_index(tick_math::MAX_TICK, self.tick_spacing);
            // find the next tick array start index
            max_tick_boundary += TickArrayState::tick_count(self.tick_spacing);
        }
        if min_tick_boundary < tick_math::MIN_TICK {
            min_tick_boundary =
                TickArrayState::get_array_start_index(tick_math::MIN_TICK, self.tick_spacing);
        }
        (min_tick_boundary, max_tick_boundary)
    }
}

/// The pool's tick array bitmap beyond the default bitmap's range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickArrayBitmapExtension {
    pub pool_id: Pubkey,
    /// Packed initialized tick array state for start_tick_index is positive
    pub positive_tick_array_bitmap: [[u64; 8]; EXTENSION_TICKARRAY_BITMAP_SIZE],
    /// Packed initialized tick array state for start_tick_index is negitive
    pub negative_tick_array_bitmap: [[u64; 8]; EXTENSION_TICKARRAY_BITMAP_SIZE],
}

impl TickArrayBitmapExtension {
    pub const LEN: usize = 8 + 32 + 64 * EXTENSION_TICKARRAY_BITMAP_SIZE * 2;

    /// Layout: discriminator, pool_id @8, positive bitmaps @40, negative bitmaps @936.
    pub fn decode(data: &[u8]) -> Result<Self> {
        if data.len() != Self::LEN || data[..8] != TICK_ARRAY_BITMAP_EXTENSION_DISCRIMINATOR {
            return Err(ErrorCode::InvalidAccountData);
        }
        let bitmaps = |start: usize| {
            let mut out = [[0u64; 8]; EXTENSION_TICKARRAY_BITMAP_SIZE];
            for (i, bitmap) in out.iter_mut().enumerate() {
                for (j, word) in bitmap.iter_mut().enumerate() {
                    *word = u64_at(data, start + 64 * i + 8 * j);
                }
            }
            out
        };
        Ok(Self {
            pool_id: pubkey_at(data, 8),
            positive_tick_array_bitmap: bitmaps(40),
            negative_tick_array_bitmap: bitmaps(40 + 64 * EXTENSION_TICKARRAY_BITMAP_SIZE),
        })
    }

    fn get_bitmap_offset(tick_index: i32, tick_spacing: u16) -> Result<usize> {
        require!(
            TickArrayState::check_is_valid_start_index(tick_index, tick_spacing),
            ErrorCode::InvalidTickIndex
        );
        Self::check_extension_boundary(tick_index, tick_spacing)?;
        let ticks_in_one_bitmap = max_tick_in_tickarray_bitmap(tick_spacing);
        let mut offset = tick_index.abs() / ticks_in_one_bitmap - 1;
        if tick_index < 0 && tick_index.abs() % ticks_in_one_bitmap == 0 {
            offset -= 1;
        }
        Ok(offset as usize)
    }

    /// According to the given tick, calculate its corresponding tickarray and then find the bitmap it belongs to.
    fn get_bitmap(&self, tick_index: i32, tick_spacing: u16) -> Result<(usize, TickArryBitmap)> {
        let offset = Self::get_bitmap_offset(tick_index, tick_spacing)?;
        if tick_index < 0 {
            Ok((offset, self.negative_tick_array_bitmap[offset]))
        } else {
            Ok((offset, self.positive_tick_array_bitmap[offset]))
        }
    }

    /// Check if the tick in tick array bitmap extension
    pub fn check_extension_boundary(tick_index: i32, tick_spacing: u16) -> Result<()> {
        let positive_tick_boundary = max_tick_in_tickarray_bitmap(tick_spacing);
        let negative_tick_boundary = -positive_tick_boundary;
        require_gt!(tick_math::MAX_TICK, positive_tick_boundary);
        require_gt!(negative_tick_boundary, tick_math::MIN_TICK);
        if tick_index >= negative_tick_boundary && tick_index < positive_tick_boundary {
            return err!(ErrorCode::InvalidTickArrayBoundary);
        }
        Ok(())
    }

    /// Check if the tick array is initialized
    pub fn check_tick_array_is_initialized(
        &self,
        tick_array_start_index: i32,
        tick_spacing: u16,
    ) -> Result<(bool, i32)> {
        let (_, tickarray_bitmap) = self.get_bitmap(tick_array_start_index, tick_spacing)?;

        let tick_array_offset_in_bitmap =
            Self::tick_array_offset_in_bitmap(tick_array_start_index, tick_spacing);

        if U512(tickarray_bitmap).bit(tick_array_offset_in_bitmap as usize) {
            return Ok((true, tick_array_start_index));
        }
        Ok((false, tick_array_start_index))
    }

    /// Search for the first initialized bit in bitmap according to the direction, if found return ture and the tick array start index,
    /// if not, return false and tick boundary index
    pub fn next_initialized_tick_array_from_one_bitmap(
        &self,
        last_tick_array_start_index: i32,
        tick_spacing: u16,
        zero_for_one: bool,
    ) -> Result<(bool, i32)> {
        let multiplier = TickArrayState::tick_count(tick_spacing);
        let next_tick_array_start_index = if zero_for_one {
            last_tick_array_start_index - multiplier
        } else {
            last_tick_array_start_index + multiplier
        };
        let min_tick_array_start_index =
            TickArrayState::get_array_start_index(tick_math::MIN_TICK, tick_spacing);
        let max_tick_array_start_index =
            TickArrayState::get_array_start_index(tick_math::MAX_TICK, tick_spacing);

        if next_tick_array_start_index < min_tick_array_start_index
            || next_tick_array_start_index > max_tick_array_start_index
        {
            return Ok((false, next_tick_array_start_index));
        }

        let (_, tickarray_bitmap) = self.get_bitmap(next_tick_array_start_index, tick_spacing)?;

        Ok(Self::next_initialized_tick_array_in_bitmap(
            tickarray_bitmap,
            next_tick_array_start_index,
            tick_spacing,
            zero_for_one,
        ))
    }

    pub fn next_initialized_tick_array_in_bitmap(
        tickarray_bitmap: TickArryBitmap,
        next_tick_array_start_index: i32,
        tick_spacing: u16,
        zero_for_one: bool,
    ) -> (bool, i32) {
        let (bitmap_min_tick_boundary, bitmap_max_tick_boundary) =
            get_bitmap_tick_boundary(next_tick_array_start_index, tick_spacing);

        let tick_array_offset_in_bitmap =
            Self::tick_array_offset_in_bitmap(next_tick_array_start_index, tick_spacing);
        if zero_for_one {
            // tick from upper to lower
            // find from highter bits to lower bits
            let offset_bit_map = U512(tickarray_bitmap)
                << (TICK_ARRAY_BITMAP_SIZE - 1 - tick_array_offset_in_bitmap) as usize;

            let next_bit = if offset_bit_map.is_zero() {
                None
            } else {
                Some(u16::try_from(offset_bit_map.leading_zeros()).unwrap())
            };

            if let Some(next_bit) = next_bit {
                let next_array_start_index = next_tick_array_start_index
                    - i32::from(next_bit) * TickArrayState::tick_count(tick_spacing);
                (true, next_array_start_index)
            } else {
                // not found til to the end
                (false, bitmap_min_tick_boundary)
            }
        } else {
            // tick from lower to upper
            // find from lower bits to highter bits
            let offset_bit_map = U512(tickarray_bitmap) >> tick_array_offset_in_bitmap as usize;

            let next_bit = if offset_bit_map.is_zero() {
                None
            } else {
                Some(u16::try_from(offset_bit_map.trailing_zeros()).unwrap())
            };
            if let Some(next_bit) = next_bit {
                let next_array_start_index = next_tick_array_start_index
                    + i32::from(next_bit) * TickArrayState::tick_count(tick_spacing);
                (true, next_array_start_index)
            } else {
                // not found til to the end
                (false, bitmap_max_tick_boundary - TickArrayState::tick_count(tick_spacing))
            }
        }
    }

    pub fn tick_array_offset_in_bitmap(tick_array_start_index: i32, tick_spacing: u16) -> i32 {
        let m = tick_array_start_index.abs() % max_tick_in_tickarray_bitmap(tick_spacing);
        let mut tick_array_offset_in_bitmap = m / TickArrayState::tick_count(tick_spacing);
        if tick_array_start_index < 0 && m != 0 {
            tick_array_offset_in_bitmap = TICK_ARRAY_BITMAP_SIZE - tick_array_offset_in_bitmap;
        }
        tick_array_offset_in_bitmap
    }
}
