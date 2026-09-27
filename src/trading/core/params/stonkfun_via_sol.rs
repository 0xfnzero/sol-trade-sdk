//! SOL ↔ stock-quote ↔ meme routing for StonkFun.
//!
//! Most StonkFun pools are priced in a non-SOL quote (xStocks, PreStocks, STONK,
//! etc.). Wallets that only hold SOL need an atomic two-hop:
//!
//! - buy:  `SOL/WSOL → quote → meme`
//! - sell: `meme → quote → SOL/WSOL`
//!
//! [`StonkFunViaSolParams`] covers both the LaunchLab curve (inner) and graduated
//! CPMM (outer) meme legs. The SOL↔quote hop currently supports Raydium CPMM and
//! Raydium AMM v4 pools.
//!
//! # Quick start
//!
//! ```ignore
//! use sol_trade_sdk::{
//!     BuyAmount, SimpleBuyParams, StonkFunViaSolParams,
//! };
//!
//! let via = StonkFunViaSolParams::curve_with_cpmm(curve_params, wsol_stock_pool);
//! let buy = SimpleBuyParams::stonkfun_with_sol(
//!     meme_mint,
//!     BuyAmount::ExactInput(100_000_000), // 0.1 SOL
//!     via,
//!     recent_blockhash,
//!     gas_fee_strategy,
//! );
//! ```

use super::{BonkParams, RaydiumAmmV4Params, RaydiumCpmmParams};

/// Meme ↔ StonkFun-quote leg: either the LaunchLab curve or a graduated CPMM pool.
#[derive(Clone)]
pub enum StonkFunMemeLeg {
    /// Inner-curve / bonding-curve pool (`DexParamEnum::StonkFun`).
    Curve(BonkParams),
    /// Graduated external CPMM pool (`DexParamEnum::StonkFunSwap`).
    Graduated(RaydiumCpmmParams),
}

/// SOL/WSOL ↔ StonkFun-quote hop used when the wallet does not hold the quote mint.
#[derive(Clone)]
pub enum StonkFunSolHop {
    RaydiumCpmm(RaydiumCpmmParams),
    RaydiumAmmV4(RaydiumAmmV4Params),
}

impl From<RaydiumCpmmParams> for StonkFunSolHop {
    fn from(params: RaydiumCpmmParams) -> Self {
        Self::RaydiumCpmm(params)
    }
}

impl From<RaydiumAmmV4Params> for StonkFunSolHop {
    fn from(params: RaydiumAmmV4Params) -> Self {
        Self::RaydiumAmmV4(params)
    }
}

/// Pay-with-SOL / receive-SOL wrapper around an inner or graduated StonkFun leg.
///
/// Prefer the `curve_with_*` / `graduated_with_*` constructors, then pass the
/// result to [`crate::client::SimpleBuyParams::stonkfun_with_sol`] or
/// [`crate::client::SimpleSellParams::stonkfun_to_sol`].
///
/// ATA lifecycle follows the caller's account policy:
/// - `Auto` (default on the Simple helpers): create WSOL / stock-quote / meme
///   ATAs when missing; keep stock-quote across trades; unwrap WSOL back to
///   native SOL on sells so the wallet balance looks normal.
/// - `HotPathMinimal` / `AssumePrepared`: never create or close — prebuild ATAs
///   offline for lowest latency.
#[derive(Clone)]
pub struct StonkFunViaSolParams {
    pub meme_leg: StonkFunMemeLeg,
    pub sol_hop: StonkFunSolHop,
}

impl StonkFunViaSolParams {
    /// Inner-curve meme leg + arbitrary SOL hop.
    pub fn curve(meme_leg: BonkParams, sol_hop: impl Into<StonkFunSolHop>) -> Self {
        Self { meme_leg: StonkFunMemeLeg::Curve(meme_leg), sol_hop: sol_hop.into() }
    }

    /// Graduated CPMM meme leg + arbitrary SOL hop.
    pub fn graduated(meme_leg: RaydiumCpmmParams, sol_hop: impl Into<StonkFunSolHop>) -> Self {
        Self { meme_leg: StonkFunMemeLeg::Graduated(meme_leg), sol_hop: sol_hop.into() }
    }

    /// Inner curve priced in a stock quote, with a Raydium CPMM `WSOL/quote` hop.
    pub fn curve_with_cpmm(meme_leg: BonkParams, sol_quote_pool: RaydiumCpmmParams) -> Self {
        Self::curve(meme_leg, sol_quote_pool)
    }

    /// Inner curve priced in a stock quote, with a Raydium AMM v4 `WSOL/quote` hop.
    pub fn curve_with_amm_v4(meme_leg: BonkParams, sol_quote_pool: RaydiumAmmV4Params) -> Self {
        Self::curve(meme_leg, sol_quote_pool)
    }

    /// Graduated StonkFun CPMM pool, with a Raydium CPMM `WSOL/quote` hop.
    pub fn graduated_with_cpmm(
        meme_leg: RaydiumCpmmParams,
        sol_quote_pool: RaydiumCpmmParams,
    ) -> Self {
        Self::graduated(meme_leg, sol_quote_pool)
    }

    /// Graduated StonkFun CPMM pool, with a Raydium AMM v4 `WSOL/quote` hop.
    pub fn graduated_with_amm_v4(
        meme_leg: RaydiumCpmmParams,
        sol_quote_pool: RaydiumAmmV4Params,
    ) -> Self {
        Self::graduated(meme_leg, sol_quote_pool)
    }

    /// Wrap as the protocol extension enum used by buy/sell APIs.
    #[inline]
    pub fn into_extension(self) -> super::DexParamEnum {
        super::DexParamEnum::StonkFunViaSol(self)
    }
}

impl From<StonkFunViaSolParams> for super::DexParamEnum {
    fn from(params: StonkFunViaSolParams) -> Self {
        Self::StonkFunViaSol(params)
    }
}
