//! Explicit conversion routes shared by StonkFun, PumpSwap and Pump.fun curves.
use super::raydium_cpmm::TokenTransferFee;
use super::{
    MeteoraDlmmParams, RaydiumAmmV4Params, RaydiumClmmParams, RaydiumCpmmParams, WhirlpoolParams,
};
use solana_sdk::pubkey::Pubkey;

#[derive(Clone)]
pub enum StonkFunQuoteVenue {
    /// Compact V3 inner curve. The hop minimum overrides params.minimum_output_amount.
    PumpFunV3 {
        mint: Pubkey,
        params: super::PumpFunV3Params,
    },
    /// Pump.fun bonding curve, not PumpSwap or LaunchLab. Supply a current
    /// curve snapshot and verified zero transfer-fee snapshots for both mints.
    /// Use the quote mint's actual owner (SPL Token or Token-2022).
    PumpFun {
        mint: Pubkey,
        curve: super::PumpFunParams,
        quote_token_program: Pubkey,
        base_transfer_fee: TokenTransferFee,
        quote_transfer_fee: TokenTransferFee,
    },
    /// PumpSwap legacy exact-input leg. Supply current fee snapshots for both
    /// mints; transfer-fee-bearing PumpSwap routes are not supported here.
    PumpSwap {
        pool: super::PumpSwapParams,
        base_transfer_fee: TokenTransferFee,
        quote_transfer_fee: TokenTransferFee,
    },
    RaydiumCpmm(RaydiumCpmmParams),
    RaydiumAmmV4(RaydiumAmmV4Params),
    RaydiumClmm(RaydiumClmmParams),
    OrcaWhirlpool(WhirlpoolParams),
    MeteoraDlmm(MeteoraDlmmParams),
}

/// Explicit funding conversion plus PumpSwap legs, executed atomically.
pub type PumpSwapQuoteRoute = StonkFunQuoteRoute;
pub type PumpSwapQuoteHop = StonkFunQuoteHop;
pub type PumpSwapQuoteVenue = StonkFunQuoteVenue;

/// Explicit funding conversion plus a Pump.fun bonding-curve trade.
pub type PumpFunQuoteRoute = StonkFunQuoteRoute;
pub type PumpFunQuoteHop = StonkFunQuoteHop;
pub type PumpFunQuoteVenue = StonkFunQuoteVenue;

/// One exact-input swap. Obtain fresh min-out quotes and amount-appropriate
/// tick/bin arrays before building. Historical parser amounts are not quotes.
#[derive(Clone)]
pub struct StonkFunQuoteHop {
    pub venue: StonkFunQuoteVenue,
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    /// None spends this mint's entire guaranteed available route balance.
    /// Some splits a fixed portion; subsequent branches may consume the rest.
    pub input_amount: Option<u64>,
    /// Recipient credit after output transfer fees, matching the on-chain threshold.
    pub minimum_amount_out: u64,
    /// Active output-mint fee snapshot, required for Token-2022 outputs, even
    /// when zero. Refresh across epochs/config changes. Conservative net credit
    /// is used between hops; unspent intermediate tokens remain in the wallet.
    pub output_transfer_fee: Option<TokenTransferFee>,
}
impl StonkFunQuoteHop {
    pub fn exact_in(
        venue: StonkFunQuoteVenue,
        input_mint: Pubkey,
        output_mint: Pubkey,
        minimum_amount_out: u64,
    ) -> Self {
        Self {
            venue,
            input_mint,
            output_mint,
            input_amount: None,
            minimum_amount_out,
            output_transfer_fee: None,
        }
    }
    pub fn with_input_amount(mut self, amount: u64) -> Self {
        self.input_amount = Some(amount);
        self
    }
    pub fn with_output_transfer_fee(mut self, fee: TokenTransferFee) -> Self {
        self.output_transfer_fee = Some(fee);
        self
    }

    /// Load the active output-mint fee snapshot while preparing a fresh quote.
    /// This does not quote the swap or discover a pool.
    pub async fn with_output_transfer_fee_by_rpc(
        mut self,
        rpc: &crate::common::SolanaRpcClient,
    ) -> anyhow::Result<Self> {
        let mint = rpc.get_account(&self.output_mint).await?;
        let epoch = rpc.get_epoch_info().await?.epoch;
        self.output_transfer_fee =
            Some(super::raydium_cpmm::token_transfer_fee_for_epoch(&mint.data, mint.owner, epoch)?);
        Ok(self)
    }
}

/// Buy conversion (SOL → quote) and independently prepared sell conversion
/// (quote → SOL). Each is an execution-ordered list, supporting intermediate
/// mints, splits and merges. A sell route needs fresh direction-specific arrays
/// and quotes; the SDK never blindly reverses a buy route.
#[derive(Clone, Default)]
pub struct StonkFunQuoteRoute {
    pub buy_hops: Vec<StonkFunQuoteHop>,
    pub sell_hops: Vec<StonkFunQuoteHop>,
    /// Auto prefers official Pump execution only with an explicit official final limit.
    pub buy_execution: crate::PumpRouteOptions,
    pub sell_execution: crate::PumpRouteOptions,
}

/// Preflight amounts and account requirements for an explicitly quoted route.
#[derive(Clone, Debug)]
pub struct StonkFunQuoteRoutePreview {
    pub hops: Vec<StonkFunQuoteHopPreview>,
    pub minimum_output_credit: u64,
}

#[derive(Clone, Debug)]
pub struct StonkFunQuoteHopPreview {
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub input_token_program: Pubkey,
    pub output_token_program: Pubkey,
    pub input_amount: u64,
    pub minimum_amount_out: u64,
}

impl StonkFunQuoteRoute {
    /// Preview the sequential hop plan without RPC or instructions.
    /// This is not an official multi_hop_swap quote; execution options are not applied.
    /// Pool state and quotes must be fresh.
    pub fn preview(
        &self,
        input_mint: Pubkey,
        output_mint: Pubkey,
        input_amount: u64,
        slippage_basis_points: u64,
        is_buy: bool,
    ) -> anyhow::Result<StonkFunQuoteRoutePreview> {
        let plan = crate::instruction::stonkfun_quote_route::plan_route(
            self,
            input_mint,
            output_mint,
            input_amount,
            slippage_basis_points,
            is_buy,
        )?;
        Ok(StonkFunQuoteRoutePreview {
            minimum_output_credit: plan.minimum_credit,
            hops: plan
                .hops
                .iter()
                .map(|hop| StonkFunQuoteHopPreview {
                    input_mint: hop.input,
                    output_mint: hop.output,
                    input_token_program: hop.input_program,
                    output_token_program: hop.output_program,
                    input_amount: hop.amount,
                    minimum_amount_out: hop.hop.minimum_amount_out,
                })
                .collect(),
        })
    }
    pub fn buy(hops: Vec<StonkFunQuoteHop>) -> Self {
        Self { buy_hops: hops, sell_hops: Vec::new(), ..Self::default() }
    }
    pub fn sell(hops: Vec<StonkFunQuoteHop>) -> Self {
        Self { buy_hops: Vec::new(), sell_hops: hops, ..Self::default() }
    }
    pub fn with_sell_hops(mut self, hops: Vec<StonkFunQuoteHop>) -> Self {
        self.sell_hops = hops;
        self
    }

    pub fn with_buy_execution(mut self, options: crate::PumpRouteOptions) -> Self {
        self.buy_execution = options;
        self
    }

    pub fn with_sell_execution(mut self, options: crate::PumpRouteOptions) -> Self {
        self.sell_execution = options;
        self
    }
}
