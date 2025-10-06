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
    ceil_div(amount * fee_basis_points, 10_000)
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
    (a + b - 1) / b
}

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
#[inline(always)]
pub const fn calculate_with_slippage_buy(amount: u64, basis_points: u64) -> u64 {
    amount + (amount * basis_points / 10000)
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
    if amount <= basis_points / 10000 {
        1
    } else {
        amount - (amount * basis_points / 10000)
    }
}
