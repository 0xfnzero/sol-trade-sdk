use anyhow::{anyhow, Result};
use solana_sdk::pubkey::Pubkey;

use super::HopSpot;
use crate::common::SolanaRpcClient;

/// Orca Whirlpool `swap_v2` parameters (exact-in).
#[derive(Clone, Debug)]
pub struct WhirlpoolParams {
    pub whirlpool: Pubkey,
    pub mint_a: Pubkey,
    pub mint_b: Pubkey,
    pub vault_a: Pubkey,
    pub vault_b: Pubkey,
    pub token_program_a: Pubkey,
    pub token_program_b: Pubkey,
    pub tick_arrays: Vec<Pubkey>,
    /// `0` → Orca full-range MIN/MAX for direction.
    pub sqrt_price_limit: u128,
    /// Spot price and fee when loaded; quotes a swap through the pool.
    pub spot: Option<HopSpot>,
}

impl WhirlpoolParams {
    pub fn new(
        whirlpool: Pubkey,
        mint_a: Pubkey,
        mint_b: Pubkey,
        vault_a: Pubkey,
        vault_b: Pubkey,
        token_program_a: Pubkey,
        token_program_b: Pubkey,
        tick_arrays: Vec<Pubkey>,
    ) -> Self {
        Self {
            whirlpool,
            mint_a,
            mint_b,
            vault_a,
            vault_b,
            token_program_a,
            token_program_b,
            tick_arrays,
            sqrt_price_limit: 0,
            spot: None,
        }
    }

    pub fn with_spot(mut self, spot: HopSpot) -> Self {
        self.spot = Some(spot);
        self
    }

    pub fn with_sqrt_price_limit(mut self, limit: u128) -> Self {
        self.sqrt_price_limit = limit;
        self
    }

    /// Pool accounts, spot price and the tick arrays of an `input_mint →
    /// output_mint` swap, in two RPC round trips.
    pub async fn from_pool_address_by_rpc(
        rpc: &SolanaRpcClient,
        whirlpool: &Pubkey,
        input_mint: &Pubkey,
        output_mint: &Pubkey,
    ) -> Result<Self> {
        use crate::instruction::utils::whirlpool::{
            decode_whirlpool, resolve_tick_arrays_for_swap, PROGRAM_ID,
        };
        let accounts = rpc.get_multiple_accounts(&[*whirlpool, *input_mint, *output_mint]).await?;
        let pool = accounts[0]
            .as_ref()
            .filter(|account| account.owner == PROGRAM_ID)
            .ok_or_else(|| anyhow!("{whirlpool} is not a Whirlpool"))?;
        let state = decode_whirlpool(&pool.data)?;
        let a_to_b = if input_mint == &state.token_mint_a && output_mint == &state.token_mint_b {
            true
        } else if input_mint == &state.token_mint_b && output_mint == &state.token_mint_a {
            false
        } else {
            anyhow::bail!("Whirlpool swap mints do not match pool");
        };
        let program = |index: usize| {
            accounts[index]
                .as_ref()
                .map(|account| account.owner)
                .ok_or_else(|| anyhow!("Whirlpool mint account missing"))
        };
        let (input_program, output_program) = (program(1)?, program(2)?);
        let (token_program_a, token_program_b) = if a_to_b {
            (input_program, output_program)
        } else {
            (output_program, input_program)
        };
        let tick_arrays = resolve_tick_arrays_for_swap(
            rpc,
            whirlpool,
            state.tick_current_index,
            state.tick_spacing,
            a_to_b,
        )
        .await?;
        Ok(Self {
            whirlpool: *whirlpool,
            mint_a: state.token_mint_a,
            mint_b: state.token_mint_b,
            vault_a: state.token_vault_a,
            vault_b: state.token_vault_b,
            token_program_a,
            token_program_b,
            tick_arrays,
            sqrt_price_limit: 0,
            spot: Some(state.spot()),
        })
    }
}
