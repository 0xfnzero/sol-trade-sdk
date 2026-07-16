/// Calculate transaction fee based on amount and fee basis points
///
/// # Parameters
/// * `amount` - Transaction amount
/// * `fee_basis_points` - Fee basis points, 1 basis point = 0.01%
///
/// # Examples
/// * fee_basis_points = 1   -> 0.01% fee
/// * fee_basis_points = 10  -> 0.1% fee
/// * fee_basis_points = 25  -> 0.25% fee (common exchange rate)
/// * fee_basis_points = 100 -> 1% fee
#[inline(always)]
pub const fn compute_fee(amount: u128, fee_basis_points: u128) -> u128 {
    let whole = match (amount / 10_000).checked_mul(fee_basis_points) {
        Some(value) => value,
        None => return u128::MAX,
    };
    let remainder_product = match (amount % 10_000).checked_mul(fee_basis_points) {
        Some(value) => value,
        None => return u128::MAX,
    };
    match whole.checked_add(ceil_div(remainder_product, 10_000)) {
        Some(value) => value,
        None => u128::MAX,
    }
}

/// Ceiling division implementation
/// Ceiling division that ensures results are not lost due to integer division precision
///
/// # Parameters
/// * `a` - Dividend
/// * `b` - Divisor
///
/// # Returns
/// Returns the ceiling result of a/b
#[inline(always)]
pub const fn ceil_div(a: u128, b: u128) -> u128 {
    let quotient = a / b;
    if a % b == 0 {
        quotient
    } else {
        quotient + 1
    }
}

/// Maximum slippage in basis points (99.99% = 9999 bps)
/// This prevents the wrap amount from doubling when slippage is 100%
pub const MAX_SLIPPAGE_BASIS_POINTS: u64 = 9999;

/// Calculate buy amount with slippage protection
/// Add slippage percentage to the amount to ensure successful purchase
///
/// # Parameters
/// * `amount` - Original transaction amount
/// * `basis_points` - Slippage basis points, 1 basis point = 0.01%
///
/// # Examples
/// * basis_points = 1   -> 0.01% slippage
/// * basis_points = 10  -> 0.1% slippage
/// * basis_points = 100 -> 1% slippage
/// * basis_points = 500 -> 5% slippage
///
/// # Note
/// Basis points are clamped to MAX_SLIPPAGE_BASIS_POINTS (9999 = 99.99%)
/// to prevent the amount from doubling when basis_points = 10000.
#[inline(always)]
pub const fn calculate_with_slippage_buy(amount: u64, basis_points: u64) -> u64 {
    let bps = if basis_points > MAX_SLIPPAGE_BASIS_POINTS {
        MAX_SLIPPAGE_BASIS_POINTS
    } else {
        basis_points
    };
    let result = amount as u128 + (amount as u128 * bps as u128 / 10_000);
    if result > u64::MAX as u128 {
        u64::MAX
    } else {
        result as u64
    }
}

/// Calculate sell amount with slippage protection
/// Subtract slippage percentage from the amount to ensure successful sale
///
/// # Parameters
/// * `amount` - Original transaction amount
/// * `basis_points` - Slippage basis points, 1 basis point = 0.01%
///
/// # Examples
/// * basis_points = 1   -> 0.01% slippage
/// * basis_points = 10  -> 0.1% slippage
/// * basis_points = 100 -> 1% slippage
/// * basis_points = 500 -> 5% slippage
#[inline(always)]
pub const fn calculate_with_slippage_sell(amount: u64, basis_points: u64) -> u64 {
    if amount == 0 {
        return 0;
    }
    let bps = if basis_points > MAX_SLIPPAGE_BASIS_POINTS {
        MAX_SLIPPAGE_BASIS_POINTS
    } else {
        basis_points
    };
    amount - (amount as u128 * bps as u128 / 10_000) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ceil_div_handles_u128_max_without_addition_overflow() {
        assert_eq!(ceil_div(u128::MAX, u128::MAX), 1);
        assert_eq!(ceil_div(u128::MAX, 2), u128::MAX / 2 + 1);
    }

    #[test]
    fn compute_fee_handles_large_amounts_without_multiplication_overflow() {
        assert_eq!(compute_fee(u128::MAX, 1), ceil_div(u128::MAX, 10_000));
        assert_eq!(compute_fee(u128::MAX, 10_000), u128::MAX);
    }

    #[test]
    fn slippage_helpers_are_deterministic_at_numeric_boundaries() {
        assert_eq!(calculate_with_slippage_buy(u64::MAX, 100), u64::MAX);
        assert_eq!(calculate_with_slippage_sell(u64::MAX, 100), 18_262_276_632_972_456_099);
        assert_eq!(calculate_with_slippage_sell(10_000, u64::MAX), 1);
        assert_eq!(calculate_with_slippage_sell(1, u64::MAX), 1);
    }
}
