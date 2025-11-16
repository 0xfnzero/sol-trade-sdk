use anyhow::{Context, Result};
use num_traits::FromPrimitive;
use crate::instruction::utils::meteora_dlmm::math::{mul_div, mul_shr, shl_div, Rounding};

#[inline]
pub fn safe_mul_shr_cast<T: FromPrimitive>(
    x: u128,
    y: u128,
    offset: u8,
    rounding: Rounding,
) -> Result<T> {
    T::from_u128(mul_shr(x, y, offset, rounding).context("overflow")?).context("overflow")
}

#[inline]
pub fn safe_shl_div_cast<T: FromPrimitive>(
    x: u128,
    y: u128,
    offset: u8,
    rounding: Rounding,
) -> Result<T> {
    T::from_u128(shl_div(x, y, offset, rounding).context("overflow")?).context("overflow")
}

pub fn safe_mul_div_cast<T: FromPrimitive>(
    x: u128,
    y: u128,
    denominator: u128,
    rounding: Rounding,
) -> Result<T> {
    T::from_u128(mul_div(x, y, denominator, rounding).context("overflow")?).context("overflow")
}
