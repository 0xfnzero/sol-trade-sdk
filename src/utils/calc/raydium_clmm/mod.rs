//! Exact Raydium CLMM swap quotes.
//!
//! The math modules are the program's own libraries, vendored from
//! <https://github.com/raydium-io/raydium-clmm> (Apache-2.0, see `LICENSE` here).
//! `state` decodes the accounts a swap reads and ports the tick array, bitmap and
//! dynamic fee logic; `quote` runs the program's swap loop over them.

// Stand-ins for the Anchor macros the vendored code uses, returning the local
// `ErrorCode`. Defined before the modules so they are in scope inside them.
macro_rules! require {
    ($cond:expr, $err:expr) => {
        if !($cond) {
            return Err($err);
        }
    };
}

macro_rules! require_eq {
    ($left:expr, $right:expr) => {
        if $left != $right {
            return Err($crate::utils::calc::raydium_clmm::error::ErrorCode::RequireViolated);
        }
    };
    ($left:expr, $right:expr, $err:expr) => {
        if $left != $right {
            return Err($err);
        }
    };
}

macro_rules! require_gt {
    ($left:expr, $right:expr) => {
        if $left <= $right {
            return Err($crate::utils::calc::raydium_clmm::error::ErrorCode::RequireViolated);
        }
    };
    ($left:expr, $right:expr, $err:expr) => {
        if $left <= $right {
            return Err($err);
        }
    };
}

macro_rules! require_gte {
    ($left:expr, $right:expr) => {
        if $left < $right {
            return Err($crate::utils::calc::raydium_clmm::error::ErrorCode::RequireViolated);
        }
    };
    ($left:expr, $right:expr, $err:expr) => {
        if $left < $right {
            return Err($err);
        }
    };
}

macro_rules! err {
    ($err:expr) => {
        Err($err)
    };
}

macro_rules! error {
    ($err:expr) => {
        $err
    };
}

pub mod big_num;
pub mod config;
pub mod error;
pub mod quote;
pub mod state;

// Vendored as the program has them; their style is left alone.
#[allow(clippy::all)]
pub mod fixed_point_64;
#[allow(clippy::all)]
pub mod full_math;
#[allow(clippy::all)]
pub mod liquidity_math;
#[allow(clippy::all)]
pub mod sqrt_price_math;
#[allow(clippy::all)]
pub mod swap_math;
#[allow(clippy::all)]
pub mod tick_array_bit_map;
#[allow(clippy::all)]
pub mod tick_math;
#[allow(clippy::all)]
pub mod unsafe_math;

#[cfg(test)]
mod fixture_tests;

// The vendored modules import each other through the parent, as in the
// program's `libraries/mod.rs`.
pub use big_num::*;
pub use fixed_point_64::*;
pub use full_math::*;
pub use liquidity_math::*;
pub use sqrt_price_math::*;
pub use swap_math::*;
pub use tick_array_bit_map::*;
pub use tick_math::*;
pub use unsafe_math::*;
