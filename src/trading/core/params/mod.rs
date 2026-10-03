//! DEX protocol parameter types and [`SwapParams`].

mod bonk;
mod dex_swap;
mod hop_spot;
mod meteora_damm_v2;
mod meteora_dbc;
mod meteora_dlmm;
mod pumpfun;
mod pumpswap;
mod raydium_amm_v4;
mod raydium_clmm;
mod raydium_cpmm;
mod stonkfun_via_sol;
mod whirlpool;

pub use bonk::{BonkParams, LaunchLabParams, StonkFunParams};
pub use dex_swap::{DexParamEnum, SenderConcurrencyConfig, SwapParams};
pub use hop_spot::HopSpot;
pub use meteora_damm_v2::MeteoraDammV2Params;
pub use meteora_dbc::{DbcQuoteState, DbcTransferHook, MeteoraDbcParams};
#[cfg(test)]
pub(crate) use meteora_dlmm::fixture_pair as dlmm_fixture_pair;
pub use meteora_dlmm::{
    DlmmHopQuote, DlmmQuoteAccounts, DlmmQuoteState, MeteoraDlmmParams, QUOTE_BIN_ARRAYS,
};
pub use pumpfun::PumpFunParams;
pub use pumpswap::PumpSwapParams;
pub use raydium_amm_v4::RaydiumAmmV4Params;
pub use raydium_clmm::{
    ClmmHopQuote, ClmmQuoteAccounts, ClmmQuoteState, RaydiumClmmParams, CLOCK_SYSVAR,
    QUOTE_TICK_ARRAYS,
};
pub use raydium_cpmm::{
    token_transfer_fee_for_epoch, CpmmQuoteAccounts, RaydiumCpmmParams, TokenTransferFee,
};
pub use stonkfun_via_sol::{StonkFunMemeLeg, StonkFunSolHop, StonkFunViaSolParams};
pub use whirlpool::WhirlpoolParams;
/// User-facing parameters for a graduated StonkFun pool on the external CPMM venue.
pub type StonkFunSwapParams = RaydiumCpmmParams;
