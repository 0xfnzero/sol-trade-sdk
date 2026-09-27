//! Errors of the CLMM swap math, named as in the program's `ErrorCode`.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    CalculateOverflow,
    InvalidTickArrayBoundary,
    InvalidTickIndex,
    LiquidityAddValueErr,
    LiquiditySubValueErr,
    MaxTokenOverflow,
    SqrtPriceX64,
    TickUpperOverflow,
    ZeroLiquidity,
    ZeroSqrtPrice,
    /// A bare `require_*!` check failed (Anchor's `Require*Violated`).
    RequireViolated,
    /// The pool has swaps disabled (status bit 4).
    NotApproved,
    /// The pool does not open for swaps yet.
    PoolNotOpen,
    ZeroAmountSpecified,
    /// A tick array the swap crosses is not among the decoded ones; see
    /// `quote::QuoteError`.
    NotEnoughTickArrayAccount,
    InvalidFirstTickArrayAccount,
    InvalidTickArray,
    /// The bitmap extension is needed but was not provided.
    MissingTickArrayBitmapExtensionAccount,
    /// No initialized liquidity left in the swap direction.
    LiquidityInsufficient,
    InsufficientLiquidityForDirection,
    InvalidLimitOrderAmount,
    /// The swap would fill only part of the input.
    PartialFill,
    TooSmallInputOrOutputAmount,
    /// Account data does not decode as the expected account.
    InvalidAccountData,
}

pub type Result<T> = core::result::Result<T, ErrorCode>;

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Raydium CLMM quote: {self:?}")
    }
}

impl std::error::Error for ErrorCode {}
