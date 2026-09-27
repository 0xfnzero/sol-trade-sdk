use anyhow::{anyhow, Result};
use solana_sdk::pubkey::Pubkey;

use super::HopSpot;
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
    /// Spot price and fee when loaded; quotes a swap through the pool.
    pub spot: Option<HopSpot>,
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
            spot: None,
        }
    }

    pub fn with_spot(mut self, spot: HopSpot) -> Self {
        self.spot = Some(spot);
        self
    }

    pub fn with_bitmap_extension(mut self, ext: Pubkey) -> Self {
        self.tick_array_bitmap_extension = Some(ext);
        self
    }

    pub fn with_sqrt_price_limit(mut self, limit: u128) -> Self {
        self.sqrt_price_limit_x64 = limit;
        self
    }

    /// Pool accounts, spot price and the tick arrays of an `input_mint →
    /// output_mint` swap, in two RPC round trips.
    pub async fn from_pool_address_by_rpc(
        rpc: &SolanaRpcClient,
        pool: &Pubkey,
        input_mint: &Pubkey,
        output_mint: &Pubkey,
    ) -> Result<Self> {
        use crate::instruction::utils::raydium_clmm::{
            decode_amm_config_trade_fee_rate, decode_pool_state, initialized_tick_arrays,
            tick_array_bitmap_extension, tick_array_candidates, PROGRAM_ID,
        };
        let bitmap = tick_array_bitmap_extension(pool);
        let accounts =
            rpc.get_multiple_accounts(&[*pool, *input_mint, *output_mint, bitmap]).await?;
        let pool_account = accounts[0]
            .as_ref()
            .filter(|account| account.owner == PROGRAM_ID)
            .ok_or_else(|| anyhow!("{pool} is not a Raydium CLMM pool"))?;
        let state = decode_pool_state(&pool_account.data)?;
        let zero_for_one = if input_mint == &state.token_mint_0 && output_mint == &state.token_mint_1
        {
            true
        } else if input_mint == &state.token_mint_1 && output_mint == &state.token_mint_0 {
            false
        } else {
            anyhow::bail!("CLMM swap mints do not match pool");
        };
        let program = |index: usize| {
            accounts[index]
                .as_ref()
                .map(|account| account.owner)
                .ok_or_else(|| anyhow!("CLMM mint account missing"))
        };
        let (input_program, output_program) = (program(1)?, program(2)?);
        let (token_0_program, token_1_program) = if zero_for_one {
            (input_program, output_program)
        } else {
            (output_program, input_program)
        };
        let bitmap_extension = accounts[3].as_ref().map(|_| bitmap);

        let candidates =
            tick_array_candidates(pool, state.tick_current, state.tick_spacing, zero_for_one);
        let mut keys = candidates.clone();
        keys.push(state.amm_config);
        let mut found = rpc.get_multiple_accounts(&keys).await?;
        let config = found
            .pop()
            .flatten()
            .ok_or_else(|| anyhow!("CLMM AmmConfig {} missing", state.amm_config))?;
        let tick_arrays = initialized_tick_arrays(candidates, &found)?;
        let spot = state.spot(decode_amm_config_trade_fee_rate(&config.data)?);
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
            spot: Some(spot),
        })
    }
}
