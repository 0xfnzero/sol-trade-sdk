//! The program's big integers (`programs/amm/src/libraries/big_num.rs`), from the
//! `uint` crate its fork is based on. `U1024`, which the program builds with a
//! local macro for its bit operations only, uses `construct_uint!` too.

// Lints fire inside the `uint` macro's expansion.
#![allow(clippy::all)]

use uint::construct_uint;

construct_uint! {
    pub struct U128(2);
}

construct_uint! {
    pub struct U256(4);
}

construct_uint! {
    pub struct U512(8);
}

construct_uint! {
    pub struct U1024(16);
}
