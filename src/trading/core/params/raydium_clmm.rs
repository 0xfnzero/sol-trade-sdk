use anyhow::Result;
use solana_sdk::pubkey::Pubkey;

use crate::common::SolanaRpcClient;

/// Raydium CLMM `swap_v2` parameters (exact-in).
#[derive(Clone, Debug)]
pub struct RaydiumClmmParams {
    pub amm_config: Pubkey,
    pub pool_state: Pubkey,
    pub observation_state: Pubkey,
    pub token_0_mint: Pubkey,
    pub token_1_mint: Pubkey,
    pub token_0_vault: Pubkey,
    pub token_1_vault: Pubkey,
    pub token_0_program: Pubkey,
    pub token_1_program: Pubkey,
    pub tick_arrays: Vec<Pubkey>,
    pub tick_array_bitmap_extension: Option<Pubkey>,
    /// `0` → full-range limit derived from swap direction.
    pub sqrt_price_limit_x64: u128,
}

impl RaydiumClmmParams {
    pub fn new(
        amm_config: Pubkey,
        pool_state: Pubkey,
        observation_state: Pubkey,
        token_0_mint: Pubkey,
        token_1_mint: Pubkey,
        token_0_vault: Pubkey,
        token_1_vault: Pubkey,
        token_0_program: Pubkey,
        token_1_program: Pubkey,
        tick_arrays: Vec<Pubkey>,
    ) -> Self {
        Self {
            amm_config,
            pool_state,
            observation_state,
            token_0_mint,
            token_1_mint,
            token_0_vault,
            token_1_vault,
            token_0_program,
            token_1_program,
            tick_arrays,
            tick_array_bitmap_extension: None,
            sqrt_price_limit_x64: 0,
        }
    }

    pub fn with_bitmap_extension(mut self, ext: Pubkey) -> Self {
        self.tick_array_bitmap_extension = Some(ext);
        self
    }

    pub fn with_sqrt_price_limit(mut self, limit: u128) -> Self {
        self.sqrt_price_limit_x64 = limit;
        self
    }

    /// Load pool state + neighboring tick arrays for `input_mint → output_mint`.
    pub async fn from_pool_address_by_rpc(
        rpc: &SolanaRpcClient,
        pool: &Pubkey,
        input_mint: &Pubkey,
        output_mint: &Pubkey,
    ) -> Result<Self> {
        use crate::instruction::utils::raydium_clmm::{
            fetch_pool, resolve_tick_arrays_for_swap, tick_array_bitmap_extension,
        };
        let state = fetch_pool(rpc, pool).await?;
        let zero_for_one = if input_mint == &state.token_mint_0 && output_mint == &state.token_mint_1
        {
            true
        } else if input_mint == &state.token_mint_1 && output_mint == &state.token_mint_0 {
            false
        } else {
            anyhow::bail!("CLMM swap mints do not match pool");
        };
        let tick_arrays = resolve_tick_arrays_for_swap(
            rpc,
            pool,
            state.tick_current,
            state.tick_spacing,
            zero_for_one,
        )
        .await?;
        let mint_accounts =
            rpc.get_multiple_accounts(&[state.token_mint_0, state.token_mint_1]).await?;
        let token_0_program = mint_accounts
            .first()
            .and_then(|a| a.as_ref())
            .map(|a| a.owner)
            .ok_or_else(|| anyhow::anyhow!("token0 mint missing"))?;
        let token_1_program = mint_accounts
            .get(1)
            .and_then(|a| a.as_ref())
            .map(|a| a.owner)
            .ok_or_else(|| anyhow::anyhow!("token1 mint missing"))?;
        let bitmap = tick_array_bitmap_extension(pool);
        let bitmap_extension = match rpc.get_account(&bitmap).await {
            Ok(_) => Some(bitmap),
            Err(_) => None,
        };
        Ok(Self {
            amm_config: state.amm_config,
            pool_state: *pool,
            observation_state: state.observation_key,
            token_0_mint: state.token_mint_0,
            token_1_mint: state.token_mint_1,
            token_0_vault: state.token_vault_0,
            token_1_vault: state.token_vault_1,
            token_0_program,
            token_1_program,
            tick_arrays,
            tick_array_bitmap_extension: bitmap_extension,
            sqrt_price_limit_x64: 0,
        })
    }
}
