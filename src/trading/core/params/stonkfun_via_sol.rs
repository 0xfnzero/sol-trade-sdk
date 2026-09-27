//! SOL ↔ stock-quote ↔ meme routing for StonkFun.
//!
//! Most StonkFun pools are priced in a non-SOL quote (xStocks, PreStocks, STONK,
//! etc.). Wallets that only hold SOL need an atomic two-hop:
//!
//! - buy:  `SOL/WSOL → quote → meme`
//! - sell: `meme → quote → SOL/WSOL`
//!
//! [`StonkFunViaSolParams`] covers both the LaunchLab curve (inner) and graduated
//! CPMM (outer) meme legs. The SOL↔quote hop goes through a Raydium CPMM, Raydium
//! AMM v4, Raydium CLMM, Orca Whirlpool or Meteora DLMM pool; a quote that only
//! trades against another currency (USDC) takes a second hop from it. Hops
//! through concentrated-liquidity pools are quoted at the pool's spot price
//! ([`super::HopSpot`]), which their loaders fill in.
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

use solana_sdk::pubkey::Pubkey;

use super::{
    BonkParams, MeteoraDlmmParams, RaydiumAmmV4Params, RaydiumClmmParams, RaydiumCpmmParams,
    WhirlpoolParams,
};

/// Meme ↔ StonkFun-quote leg: either the LaunchLab curve or a graduated CPMM pool.
#[derive(Clone)]
pub enum StonkFunMemeLeg {
    /// Inner-curve / bonding-curve pool (`DexParamEnum::StonkFun`).
    Curve(BonkParams),
    /// Graduated external CPMM pool (`DexParamEnum::StonkFunSwap`).
    Graduated(RaydiumCpmmParams),
}

/// A pool on the route between SOL/WSOL and the StonkFun quote, used when the
/// wallet does not hold the quote mint.
#[derive(Clone)]
pub enum StonkFunSolHop {
    RaydiumCpmm(RaydiumCpmmParams),
    RaydiumAmmV4(RaydiumAmmV4Params),
    /// Quoted at the pool's `spot` price.
    RaydiumClmm(RaydiumClmmParams),
    /// Quoted at the pool's `spot` price.
    OrcaWhirlpool(WhirlpoolParams),
    /// Quoted at the pair's `spot` price.
    MeteoraDlmm(MeteoraDlmmParams),
}

impl StonkFunSolHop {
    /// The pool's address.
    pub fn pool(&self) -> Pubkey {
        match self {
            Self::RaydiumCpmm(pool) => pool.pool_state,
            Self::RaydiumAmmV4(pool) => pool.amm,
            Self::RaydiumClmm(pool) => pool.pool_state,
            Self::OrcaWhirlpool(pool) => pool.whirlpool,
            Self::MeteoraDlmm(pool) => pool.lb_pair,
        }
    }

    /// The pool's two mints, in the pool's own order.
    pub fn mints(&self) -> (Pubkey, Pubkey) {
        match self {
            Self::RaydiumCpmm(pool) => (pool.base_mint, pool.quote_mint),
            Self::RaydiumAmmV4(pool) => (pool.coin_mint, pool.pc_mint),
            Self::RaydiumClmm(pool) => (pool.token_0_mint, pool.token_1_mint),
            Self::OrcaWhirlpool(pool) => (pool.mint_a, pool.mint_b),
            Self::MeteoraDlmm(pool) => (pool.token_x_mint, pool.token_y_mint),
        }
    }
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

impl From<RaydiumClmmParams> for StonkFunSolHop {
    fn from(params: RaydiumClmmParams) -> Self {
        Self::RaydiumClmm(params)
    }
}

impl From<WhirlpoolParams> for StonkFunSolHop {
    fn from(params: WhirlpoolParams) -> Self {
        Self::OrcaWhirlpool(params)
    }
}

impl From<MeteoraDlmmParams> for StonkFunSolHop {
    fn from(params: MeteoraDlmmParams) -> Self {
        Self::MeteoraDlmm(params)
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
    /// The pool trading SOL: against the quote itself, or against the
    /// currency `quote_hop` trades the quote against.
    pub sol_hop: StonkFunSolHop,
    /// Second hop, between the currency `sol_hop` trades and the quote.
    pub quote_hop: Option<StonkFunSolHop>,
    /// Slippage of each hop; `None` uses the trade's slippage. On buys each leg
    /// spends the minimum output of the one before, so every basis point of hop
    /// slippage a pool does not use stays behind in that currency.
    pub hop_slippage_basis_points: Option<u64>,
}

impl StonkFunViaSolParams {
    /// Inner-curve meme leg + arbitrary SOL hop.
    pub fn curve(meme_leg: BonkParams, sol_hop: impl Into<StonkFunSolHop>) -> Self {
        Self {
            meme_leg: StonkFunMemeLeg::Curve(meme_leg),
            sol_hop: sol_hop.into(),
            quote_hop: None,
            hop_slippage_basis_points: None,
        }
    }

    /// Graduated CPMM meme leg + arbitrary SOL hop.
    pub fn graduated(meme_leg: RaydiumCpmmParams, sol_hop: impl Into<StonkFunSolHop>) -> Self {
        Self {
            meme_leg: StonkFunMemeLeg::Graduated(meme_leg),
            sol_hop: sol_hop.into(),
            quote_hop: None,
            hop_slippage_basis_points: None,
        }
    }

    /// Slippage of the hops, separate from the meme leg's.
    pub fn with_hop_slippage_basis_points(mut self, basis_points: u64) -> Self {
        self.hop_slippage_basis_points = Some(basis_points);
        self
    }

    /// A second hop from the currency `sol_hop` trades to the quote, for a
    /// quote that does not trade against SOL.
    pub fn with_quote_hop(mut self, quote_hop: impl Into<StonkFunSolHop>) -> Self {
        self.quote_hop = Some(quote_hop.into());
        self
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
