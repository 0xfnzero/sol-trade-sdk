//! Explicit, externally quoted StonkFun quote-conversion routes.
use super::raydium_cpmm::TokenTransferFee;
use super::{
    MeteoraDlmmParams, RaydiumAmmV4Params, RaydiumClmmParams, RaydiumCpmmParams, WhirlpoolParams,
};
use solana_sdk::pubkey::Pubkey;

#[derive(Clone)]
pub enum StonkFunQuoteVenue {
    RaydiumCpmm(RaydiumCpmmParams),
    RaydiumAmmV4(RaydiumAmmV4Params),
    RaydiumClmm(RaydiumClmmParams),
    OrcaWhirlpool(WhirlpoolParams),
    MeteoraDlmm(MeteoraDlmmParams),
}

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
    /// Validate without RPC or instructions. Pool state and quotes must be fresh.
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
        Self { buy_hops: hops, sell_hops: Vec::new() }
    }
    pub fn sell(hops: Vec<StonkFunQuoteHop>) -> Self {
        Self { buy_hops: Vec::new(), sell_hops: hops }
    }
    pub fn with_sell_hops(mut self, hops: Vec<StonkFunQuoteHop>) -> Self {
        self.sell_hops = hops;
        self
    }
}
