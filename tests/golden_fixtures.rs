//! Golden fixture test harness — aggregates per-protocol unit tests.
//! Run with: cargo test --features dev-insecure-tls --test golden_fixtures

#[path = "unit/bonk.rs"]
mod bonk;
#[path = "unit/meteora_damm_v2.rs"]
mod meteora_damm_v2;
#[path = "unit/pumpfun.rs"]
mod pumpfun;
#[path = "unit/pumpswap.rs"]
mod pumpswap;
#[path = "unit/raydium_amm_v4.rs"]
mod raydium_amm_v4;
#[path = "unit/raydium_cpmm.rs"]
mod raydium_cpmm;
