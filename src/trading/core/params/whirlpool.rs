use anyhow::Result;
use solana_sdk::pubkey::Pubkey;

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
        }
    }

    pub fn with_sqrt_price_limit(mut self, limit: u128) -> Self {
        self.sqrt_price_limit = limit;
        self
    }

    /// Load whirlpool + 3 tick arrays for `input_mint → output_mint`.
    pub async fn from_pool_address_by_rpc(
        rpc: &SolanaRpcClient,
        whirlpool: &Pubkey,
        input_mint: &Pubkey,
        output_mint: &Pubkey,
    ) -> Result<Self> {
        use crate::instruction::utils::whirlpool::{fetch_whirlpool, resolve_tick_arrays_for_swap};
        let state = fetch_whirlpool(rpc, whirlpool).await?;
        let a_to_b = if input_mint == &state.token_mint_a && output_mint == &state.token_mint_b {
            true
        } else if input_mint == &state.token_mint_b && output_mint == &state.token_mint_a {
            false
        } else {
            anyhow::bail!("Whirlpool swap mints do not match pool");
        };
        let tick_arrays = resolve_tick_arrays_for_swap(
            rpc,
            whirlpool,
            state.tick_current_index,
            state.tick_spacing,
            a_to_b,
        )
        .await?;
        let mint_accounts =
            rpc.get_multiple_accounts(&[state.token_mint_a, state.token_mint_b]).await?;
        let token_program_a = mint_accounts
            .first()
            .and_then(|a| a.as_ref())
            .map(|a| a.owner)
            .ok_or_else(|| anyhow::anyhow!("mint_a missing"))?;
        let token_program_b = mint_accounts
            .get(1)
            .and_then(|a| a.as_ref())
            .map(|a| a.owner)
            .ok_or_else(|| anyhow::anyhow!("mint_b missing"))?;
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
        })
    }
}
