use anyhow::{ensure, Context, Result};
use ruint::aliases::U1024;
use std::ops::Shl;
use std::ops::Shr;
use solana_streamer::streaming::event_parser::protocols::meteora_dlmm::types::LbPair;
use crate::instruction::utils::meteora_dlmm::constants::{BASIS_POINT_MAX, BIN_ARRAY_BITMAP_SIZE, FEE_PRECISION, MAX_BIN_ID, MAX_FEE_RATE, MIN_BIN_ID};

pub enum PairStatus {
    Enabled,
    Disabled,
}

pub enum PairType {
    Permissionless,
    Permission,
    CustomizablePermissionless,
    PermissionlessV2,
}

pub enum ActivationType {
    Slot,
    Timestamp,
}

pub trait LbPairExtension {
    fn bitmap_range() -> (i32, i32);
    fn get_bin_array_offset(bin_array_index: i32) -> usize;

    fn status(&self) -> Result<PairStatus>;
    fn pair_type(&self) -> Result<PairType>;
    fn activation_type(&self) -> Result<ActivationType>;
    fn compute_fee(&self, amount: u64) -> Result<u64>;
    fn get_total_fee(&self) -> Result<u128>;
    fn get_base_fee(&self) -> Result<u128>;
    fn get_variable_fee(&self) -> Result<u128>;
    fn compute_variable_fee(&self, volatility_accumulator: u32) -> Result<u128>;
    fn compute_protocol_fee(&self, fee_amount: u64) -> Result<u64>;
    fn compute_fee_from_amount(&self, amount_with_fees: u64) -> Result<u64>;
    fn is_overflow_default_bin_array_bitmap(&self, bin_array_index: i32) -> bool;
    fn next_bin_array_index_with_liquidity_internal(
        &self,
        swap_for_y: bool,
        start_array_index: i32,
    ) -> Result<(i32, bool)>;

    fn update_references(&mut self, current_timestamp: i64) -> Result<()>;
    fn update_volatility_accumulator(&mut self) -> Result<()>;
    fn advance_active_bin(&mut self, swap_for_y: bool) -> Result<()>;
}

impl LbPairExtension for LbPair {
    fn bitmap_range() -> (i32, i32) {
        (-BIN_ARRAY_BITMAP_SIZE, BIN_ARRAY_BITMAP_SIZE - 1)
    }

    fn get_bin_array_offset(bin_array_index: i32) -> usize {
        (bin_array_index + BIN_ARRAY_BITMAP_SIZE) as usize
    }

    fn status(&self) -> Result<PairStatus> {
        Ok(self.status.try_into()?)
    }

    fn pair_type(&self) -> Result<PairType> {
        Ok(self.pair_type.try_into()?)
    }

    fn activation_type(&self) -> Result<ActivationType> {
        Ok(self.activation_type.try_into()?)
    }

    fn compute_fee(&self, amount: u64) -> Result<u64> {
        let total_fee_rate = self.get_total_fee()?;
        let denominator = u128::from(FEE_PRECISION)
            .checked_sub(total_fee_rate)
            .context("overflow")?;

        // Ceil division
        let fee = u128::from(amount)
            .checked_mul(total_fee_rate)
            .context("overflow")?
            .checked_add(denominator)
            .context("overflow")?
            .checked_sub(1)
            .context("overflow")?;

        let scaled_down_fee = fee.checked_div(denominator).context("overflow")?;

        Ok(scaled_down_fee.try_into().context("overflow")?)
    }

    fn get_total_fee(&self) -> Result<u128> {
        let total_fee_rate = self
            .get_base_fee()?
            .checked_add(self.get_variable_fee()?)
            .context("overflow")?;
        let total_fee_rate_cap = std::cmp::min(total_fee_rate, MAX_FEE_RATE.into());
        Ok(total_fee_rate_cap)
    }

    fn get_base_fee(&self) -> Result<u128> {
        Ok(u128::from(self.parameters.base_factor)
            .checked_mul(self.bin_step.into())
            .context("overflow")?
            .checked_mul(10u128)
            .context("overflow")?
            .checked_mul(10u128.pow(self.parameters.base_fee_power_factor.into()))
            .context("overflow")?)
    }

    fn get_variable_fee(&self) -> Result<u128> {
        self.compute_variable_fee(self.v_parameters.volatility_accumulator)
    }

    fn compute_variable_fee(&self, volatility_accumulator: u32) -> Result<u128> {
        if self.parameters.variable_fee_control > 0 {
            let volatility_accumulator: u128 = volatility_accumulator.into();
            let bin_step: u128 = self.bin_step.into();
            let variable_fee_control: u128 = self.parameters.variable_fee_control.into();

            let square_vfa_bin = volatility_accumulator
                .checked_mul(bin_step)
                .context("overflow")?
                .checked_pow(2)
                .context("overflow")?;

            let v_fee = variable_fee_control
                .checked_mul(square_vfa_bin)
                .context("overflow")?;

            let scaled_v_fee = v_fee
                .checked_add(99_999_999_999)
                .context("overflow")?
                .checked_div(100_000_000_000)
                .context("overflow")?;

            return Ok(scaled_v_fee);
        }

        Ok(0)
    }

    fn compute_protocol_fee(&self, fee_amount: u64) -> Result<u64> {
        let protocol_fee = u128::from(fee_amount)
            .checked_mul(self.parameters.protocol_share.into())
            .context("overflow")?
            .checked_div(BASIS_POINT_MAX as u128)
            .context("overflow")?;

        Ok(protocol_fee.try_into().context("overflow")?)
    }

    fn compute_fee_from_amount(&self, amount_with_fees: u64) -> Result<u64> {
        let total_fee_rate = self.get_total_fee()?;

        let fee_amount = u128::from(amount_with_fees)
            .checked_mul(total_fee_rate)
            .context("overflow")?
            .checked_add((FEE_PRECISION - 1).into())
            .context("overflow")?;

        let scaled_down_fee = fee_amount
            .checked_div(FEE_PRECISION.into())
            .context("overflow")?;

        Ok(scaled_down_fee.try_into().context("overflow")?)
    }

    fn is_overflow_default_bin_array_bitmap(&self, bin_array_index: i32) -> bool {
        let (min_bitmap_id, max_bitmap_id) = Self::bitmap_range();
        bin_array_index > max_bitmap_id || bin_array_index < min_bitmap_id
    }

    fn next_bin_array_index_with_liquidity_internal(
        &self,
        swap_for_y: bool,
        start_array_index: i32,
    ) -> Result<(i32, bool)> {
        let bin_array_bitmap = U1024::from_limbs(self.bin_array_bitmap);
        let array_offset: usize = Self::get_bin_array_offset(start_array_index);
        let (min_bitmap_id, max_bitmap_id) = LbPair::bitmap_range();
        if swap_for_y {
            let bitmap_range: usize = max_bitmap_id
                .checked_sub(min_bitmap_id)
                .context("overflow")?
                .try_into()
                .context("overflow")?;
            let offset_bit_map =
                bin_array_bitmap.shl(bitmap_range.checked_sub(array_offset).context("overflow")?);

            if offset_bit_map.eq(&U1024::ZERO) {
                return Ok((min_bitmap_id.checked_sub(1).context("overflow")?, false));
            } else {
                let next_bit = offset_bit_map.leading_zeros();
                return Ok((
                    start_array_index
                        .checked_sub(next_bit as i32)
                        .context("overflow")?,
                    true,
                ));
            }
        } else {
            let offset_bit_map = bin_array_bitmap.shr(array_offset);
            if offset_bit_map.eq(&U1024::ZERO) {
                return Ok((max_bitmap_id.checked_add(1).context("overflow")?, false));
            } else {
                let next_bit = offset_bit_map.trailing_zeros();
                return Ok((
                    start_array_index
                        .checked_add(next_bit as i32)
                        .context("overflow")?,
                    true,
                ));
            };
        }
    }

    fn update_references(&mut self, current_timestamp: i64) -> Result<()> {
        let v_params = &mut self.v_parameters;
        let s_params = &self.parameters;

        let elapsed = current_timestamp
            .checked_sub(v_params.last_update_timestamp)
            .context("overflow")?;

        // Not high frequency trade
        if elapsed >= s_params.filter_period as i64 {
            // Update active id of last transaction
            v_params.index_reference = self.active_id;
            // filter period < t < decay_period. Decay time window.
            if elapsed < s_params.decay_period as i64 {
                let volatility_reference = v_params
                    .volatility_accumulator
                    .checked_mul(s_params.reduction_factor as u32)
                    .context("overflow")?
                    .checked_div(BASIS_POINT_MAX as u32)
                    .context("overflow")?;

                v_params.volatility_reference = volatility_reference;
            }
            // Out of decay time window
            else {
                v_params.volatility_reference = 0;
            }
        }

        Ok(())
    }

    fn update_volatility_accumulator(&mut self) -> Result<()> {
        let v_params = &mut self.v_parameters;
        let s_params = &self.parameters;

        let delta_id = i64::from(v_params.index_reference)
            .checked_sub(self.active_id.into())
            .context("overflow")?
            .unsigned_abs();

        let volatility_accumulator = u64::from(v_params.volatility_reference)
            .checked_add(
                delta_id
                    .checked_mul(BASIS_POINT_MAX as u64)
                    .context("overflow")?,
            )
            .context("overflow")?;

        v_params.volatility_accumulator = std::cmp::min(
            volatility_accumulator,
            s_params.max_volatility_accumulator.into(),
        )
            .try_into()
            .context("overflow")?;

        Ok(())
    }

    fn advance_active_bin(&mut self, swap_for_y: bool) -> Result<()> {
        let next_active_bin_id = if swap_for_y {
            self.active_id.checked_sub(1)
        } else {
            self.active_id.checked_add(1)
        }
            .context("overflow")?;

        ensure!(
            next_active_bin_id >= MIN_BIN_ID && next_active_bin_id <= MAX_BIN_ID,
            "Insufficient liquidity"
        );

        self.active_id = next_active_bin_id;

        Ok(())
    }
}
