use anyhow::{anyhow, Result};
use solana_sdk::pubkey::Pubkey;

use super::HopSpot;
use crate::common::SolanaRpcClient;

/// Meteora DLMM `swap2` parameters (exact-in).
#[derive(Clone, Debug)]
pub struct MeteoraDlmmParams {
    pub lb_pair: Pubkey,
    pub bitmap_extension: Option<Pubkey>,
    pub reserve_x: Pubkey,
    pub reserve_y: Pubkey,
    pub token_x_mint: Pubkey,
    pub token_y_mint: Pubkey,
    pub oracle: Pubkey,
    pub token_x_program: Pubkey,
    pub token_y_program: Pubkey,
    pub bin_arrays: Vec<Pubkey>,
    /// Spot price and fee when loaded; quotes a swap through the pool.
    pub spot: Option<HopSpot>,
}

impl MeteoraDlmmParams {
    pub fn new(
        lb_pair: Pubkey,
        reserve_x: Pubkey,
        reserve_y: Pubkey,
        token_x_mint: Pubkey,
        token_y_mint: Pubkey,
        oracle: Pubkey,
        token_x_program: Pubkey,
        token_y_program: Pubkey,
        bin_arrays: Vec<Pubkey>,
    ) -> Self {
        Self {
            lb_pair,
            bitmap_extension: None,
            reserve_x,
            reserve_y,
            token_x_mint,
            token_y_mint,
            oracle,
            token_x_program,
            token_y_program,
            bin_arrays,
            spot: None,
        }
    }

    pub fn with_spot(mut self, spot: HopSpot) -> Self {
        self.spot = Some(spot);
        self
    }

    pub fn with_bitmap_extension(mut self, ext: Pubkey) -> Self {
        self.bitmap_extension = Some(ext);
        self
    }

    /// Pair accounts, spot price and the bin arrays of an `input_mint →
    /// output_mint` swap, in two RPC round trips.
    pub async fn from_pool_address_by_rpc(
        rpc: &SolanaRpcClient,
        lb_pair: &Pubkey,
        input_mint: &Pubkey,
        output_mint: &Pubkey,
    ) -> Result<Self> {
        use crate::instruction::utils::meteora_dlmm::{
            bitmap_extension_pda, decode_lb_pair, resolve_bin_arrays_for_swap, PROGRAM_ID,
        };
        let bitmap = bitmap_extension_pda(lb_pair);
        let accounts =
            rpc.get_multiple_accounts(&[*lb_pair, *input_mint, *output_mint, bitmap]).await?;
        let pair = accounts[0]
            .as_ref()
            .filter(|account| account.owner == PROGRAM_ID)
            .ok_or_else(|| anyhow!("{lb_pair} is not a Meteora DLMM pair"))?;
        let state = decode_lb_pair(&pair.data)?;
        let swap_for_y =
            if input_mint == &state.token_x_mint && output_mint == &state.token_y_mint {
                true
            } else if input_mint == &state.token_y_mint && output_mint == &state.token_x_mint {
                false
            } else {
                anyhow::bail!("DLMM swap mints do not match pool");
            };
        let program = |index: usize| {
            accounts[index]
                .as_ref()
                .map(|account| account.owner)
                .ok_or_else(|| anyhow!("DLMM mint account missing"))
        };
        let (input_program, output_program) = (program(1)?, program(2)?);
        let (token_x_program, token_y_program) = if swap_for_y {
            (input_program, output_program)
        } else {
            (output_program, input_program)
        };
        let bin_arrays =
            resolve_bin_arrays_for_swap(rpc, lb_pair, state.active_id, swap_for_y).await?;
        Ok(Self {
            lb_pair: *lb_pair,
            bitmap_extension: accounts[3].as_ref().map(|_| bitmap),
            reserve_x: state.reserve_x,
            reserve_y: state.reserve_y,
            token_x_mint: state.token_x_mint,
            token_y_mint: state.token_y_mint,
            oracle: state.oracle,
            token_x_program,
            token_y_program,
            bin_arrays,
            spot: Some(state.spot()),
        })
    }
}
