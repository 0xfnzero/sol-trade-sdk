use anyhow::Result;
use solana_sdk::pubkey::Pubkey;

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
        }
    }

    pub fn with_bitmap_extension(mut self, ext: Pubkey) -> Self {
        self.bitmap_extension = Some(ext);
        self
    }

    /// Load LbPair + bin arrays for `input_mint → output_mint`.
    pub async fn from_pool_address_by_rpc(
        rpc: &SolanaRpcClient,
        lb_pair: &Pubkey,
        input_mint: &Pubkey,
        output_mint: &Pubkey,
    ) -> Result<Self> {
        use crate::instruction::utils::meteora_dlmm::{
            fetch_lb_pair, maybe_bitmap_extension, resolve_bin_arrays_for_swap,
        };
        let state = fetch_lb_pair(rpc, lb_pair).await?;
        // swap_for_y = true when selling X for Y.
        let swap_for_y =
            if input_mint == &state.token_x_mint && output_mint == &state.token_y_mint {
                true
            } else if input_mint == &state.token_y_mint && output_mint == &state.token_x_mint {
                false
            } else {
                anyhow::bail!("DLMM swap mints do not match pool");
            };
        let bin_arrays =
            resolve_bin_arrays_for_swap(rpc, lb_pair, state.active_id, swap_for_y).await?;
        let mint_accounts =
            rpc.get_multiple_accounts(&[state.token_x_mint, state.token_y_mint]).await?;
        let token_x_program = mint_accounts
            .first()
            .and_then(|a| a.as_ref())
            .map(|a| a.owner)
            .ok_or_else(|| anyhow::anyhow!("token_x mint missing"))?;
        let token_y_program = mint_accounts
            .get(1)
            .and_then(|a| a.as_ref())
            .map(|a| a.owner)
            .ok_or_else(|| anyhow::anyhow!("token_y mint missing"))?;
        Ok(Self {
            lb_pair: *lb_pair,
            bitmap_extension: maybe_bitmap_extension(rpc, lb_pair).await,
            reserve_x: state.reserve_x,
            reserve_y: state.reserve_y,
            token_x_mint: state.token_x_mint,
            token_y_mint: state.token_y_mint,
            oracle: state.oracle,
            token_x_program,
            token_y_program,
            bin_arrays,
        })
    }
}
