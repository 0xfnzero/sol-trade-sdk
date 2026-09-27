// Vendored from https://github.com/raydium-io/raydium-clmm at ed7c84a54ced59c55981780546adb0b4583dcf85
// (programs/amm/src/libraries/fixed_point_64.rs), Apache-2.0: see LICENSE in this directory.
// Changes: tests removed; formatted with this repository's rustfmt; Anchor errors
// replaced by the local `ErrorCode`.

/// A library for handling Q64.64 fixed point numbers
/// Used in sqrt_price_math.rs and liquidity_amounts.rs

pub const Q64: u128 = (u64::MAX as u128) + 1; // 2^64
pub const RESOLUTION: u8 = 64;
