//! Spot quotes for concentrated-liquidity pools.

use crate::utils::calc::common::calculate_min_amount_out;

/// 2^64, the scale of a Q64.64 square-root price.
const Q64: f64 = 18_446_744_073_709_551_616.0;

/// A pool's spot price and fee: what a swap small against the pool's depth
/// gets. A quote from it ignores the swap's own price impact, which the
/// caller's slippage has to cover.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HopSpot {
    /// Raw units of the pool's second mint one raw unit of its first mint is
    /// worth: B per A for Whirlpool, token 1 per token 0 for Raydium CLMM,
    /// Y per X for Meteora DLMM.
    pub price: f64,
    /// Share of the input the pool keeps as fees.
    pub fee: f64,
}

impl HopSpot {
    /// From a Q64.64 square-root price, as Whirlpool and Raydium CLMM keep it.
    pub fn from_sqrt_price_x64(sqrt_price_x64: u128, fee: f64) -> Self {
        let sqrt_price = sqrt_price_x64 as f64 / Q64;
        Self { price: sqrt_price * sqrt_price, fee }
    }

    /// From a Meteora DLMM bin: each bin is `bin_step` basis points above the
    /// one below it.
    pub fn from_bin(bin_id: i32, bin_step: u16, fee: f64) -> Self {
        Self { price: (1.0 + f64::from(bin_step) / 10_000.0).powi(bin_id), fee }
    }

    /// Output of an exact-in swap of `amount_in`, first mint to second or back.
    pub fn amount_out(&self, amount_in: u64, first_to_second: bool) -> u64 {
        let net = amount_in as f64 * (1.0 - self.fee);
        let out = if first_to_second { net * self.price } else { net / self.price };
        if out.is_finite() && out > 0.0 {
            out.min(u64::MAX as f64) as u64
        } else {
            0
        }
    }

    /// [`Self::amount_out`] less `slippage_basis_points`.
    pub fn min_amount_out(
        &self,
        amount_in: u64,
        first_to_second: bool,
        slippage_basis_points: u64,
    ) -> u64 {
        calculate_min_amount_out(self.amount_out(amount_in, first_to_second), slippage_basis_points)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqrt_price_spot_quotes_both_ways_after_the_fee() {
        // sqrt(4) = 2 in Q64.64: one raw A is worth four raw B.
        let spot = HopSpot::from_sqrt_price_x64(2 << 64, 0.01);
        assert!((spot.price - 4.0).abs() < 1e-12);
        assert_eq!(spot.amount_out(1_000, true), 3_960);
        assert_eq!(spot.amount_out(4_000, false), 990);
        assert_eq!(spot.min_amount_out(1_000, true, 100), 3_920);
    }

    #[test]
    fn bin_spot_compounds_the_bin_step() {
        let spot = HopSpot::from_bin(2, 100, 0.0);
        assert!((spot.price - 1.0201).abs() < 1e-12);
        assert!((HopSpot::from_bin(-1, 100, 0.0).price - 1.0 / 1.01).abs() < 1e-12);
        assert_eq!(HopSpot { price: 0.0, fee: 0.0 }.amount_out(1_000, false), 0);
    }
}
