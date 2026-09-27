use anyhow::{anyhow, Result};
use solana_sdk::pubkey::Pubkey;

use super::{token_transfer_fee_for_epoch, HopSpot, TokenTransferFee};
use crate::common::SolanaRpcClient;
use crate::instruction::utils::raydium_clmm::{
    tick_array_bitmap_extension, tick_array_pda, PROGRAM_ID,
};
use crate::utils::calc::raydium_clmm::{
    config::AmmConfig,
    quote::{quote_exact_in, QuoteError},
    state::{PoolState, TickArrayBitmapExtension, TickArrayState},
};

/// The clock sysvar: a quote needs the cluster's time and epoch.
pub const CLOCK_SYSVAR: Pubkey = solana_sdk::pubkey!("SysvarC1ock11111111111111111111111111111111");

/// Initialized tick arrays a quote reads ahead of the price, each way, by default.
pub const QUOTE_TICK_ARRAYS: usize = 3;

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
    /// Spot price and fee, for callers that price the pool without its ticks.
    pub spot: Option<HopSpot>,
    /// The pool's accounts, decoded, when loaded for exact quotes.
    pub quote_state: Option<Box<ClmmQuoteState>>,
}

/// What an exact quote through the pool reads: its accounts, decoded, and the
/// cluster time and epoch they were read at.
#[derive(Clone, Debug)]
pub struct ClmmQuoteState {
    pub pool: PoolState,
    pub config: AmmConfig,
    pub tick_arrays: Vec<TickArrayState>,
    pub bitmap_extension: Option<TickArrayBitmapExtension>,
    pub token_0_transfer_fee: TokenTransferFee,
    pub token_1_transfer_fee: TokenTransferFee,
    /// Cluster unix time of the read; the dynamic fee decays with it.
    pub unix_timestamp: u64,
}

/// An exact quote of a swap through the pool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClmmHopQuote {
    /// What the receiver gets, after any transfer fee of the output mint.
    pub amount_out: u64,
    /// The tick arrays the swap crosses, in order: the accounts `swap_v2` needs.
    pub tick_arrays: Vec<Pubkey>,
    /// Swap steps the program takes; its compute use grows with them.
    pub steps: usize,
}

/// Raw accounts a quote reads; `quote_account_keys` lists their addresses.
pub struct ClmmQuoteAccounts<'a> {
    pub pool: &'a [u8],
    pub amm_config: &'a [u8],
    pub bitmap_extension: Option<&'a [u8]>,
    /// The pool's initialized tick arrays, any order.
    pub tick_arrays: Vec<&'a [u8]>,
    /// Owner (token program) and data of each mint.
    pub token_0_mint: (Pubkey, &'a [u8]),
    pub token_1_mint: (Pubkey, &'a [u8]),
    pub clock: &'a [u8],
}

impl RaydiumClmmParams {
    #[allow(clippy::too_many_arguments)]
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
            quote_state: None,
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

    /// Exact quote of `amount_in` of `input_mint` through the pool, as `swap_v2`
    /// without a price limit fills it: all of it, or an error.
    pub fn quote_exact_in(&self, input_mint: &Pubkey, amount_in: u64) -> Result<ClmmHopQuote> {
        let state = self.quote_state.as_ref().ok_or_else(|| {
            anyhow!("CLMM pool {} was not loaded with the state to quote it", self.pool_state)
        })?;
        let zero_for_one = if *input_mint == self.token_0_mint {
            true
        } else if *input_mint == self.token_1_mint {
            false
        } else {
            return Err(anyhow!("{input_mint} is not a mint of CLMM pool {}", self.pool_state));
        };
        let (input_fee, output_fee) = if zero_for_one {
            (state.token_0_transfer_fee, state.token_1_transfer_fee)
        } else {
            (state.token_1_transfer_fee, state.token_0_transfer_fee)
        };
        let quote = quote_exact_in(
            &state.pool,
            &state.config,
            &state.tick_arrays,
            state.bitmap_extension.as_ref(),
            amount_in - input_fee.calculate(amount_in),
            zero_for_one,
            state.unix_timestamp,
        )
        .map_err(|err| match err {
            QuoteError::MissingTickArray(start) => anyhow!(
                "CLMM pool {} quote needs tick array {} (starting at {start})",
                self.pool_state,
                tick_array_pda(&self.pool_state, start)
            ),
            QuoteError::Swap(code) => anyhow!("CLMM pool {}: {code}", self.pool_state),
        })?;
        Ok(ClmmHopQuote {
            amount_out: quote.amount_out - output_fee.calculate(quote.amount_out),
            tick_arrays: quote
                .tick_array_start_indexes
                .iter()
                .map(|start| tick_array_pda(&self.pool_state, *start))
                .collect(),
            steps: quote.steps,
        })
    }

    /// Start indexes of the first `count` initialized tick arrays a swap in the
    /// direction reads, from `pool`'s price.
    pub fn tick_array_starts_ahead(
        pool: &PoolState,
        bitmap_extension: Option<&TickArrayBitmapExtension>,
        zero_for_one: bool,
        count: usize,
    ) -> Vec<i32> {
        let mut starts = Vec::with_capacity(count);
        let Ok((_, mut start)) =
            pool.get_first_initialized_tick_array(bitmap_extension, zero_for_one)
        else {
            return starts;
        };
        while starts.len() < count {
            starts.push(start);
            match pool.next_initialized_tick_array_start_index(
                bitmap_extension,
                start,
                zero_for_one,
            ) {
                Ok(Some(next)) => start = next,
                _ => break,
            }
        }
        starts
    }

    /// Addresses of the accounts a quote of `pool` reads, in the order
    /// `ClmmQuoteAccounts` takes them: pool, config, bitmap extension, token 0
    /// and token 1 mints, the clock, then the tick arrays ahead of the price
    /// each way, per `state` (the pool as last seen).
    pub fn quote_account_keys(
        pool: &Pubkey,
        state: &PoolState,
        bitmap_extension: Option<&TickArrayBitmapExtension>,
        tick_arrays_each_way: usize,
    ) -> Vec<Pubkey> {
        let mut keys = vec![
            *pool,
            state.amm_config,
            tick_array_bitmap_extension(pool),
            state.token_mint_0,
            state.token_mint_1,
            CLOCK_SYSVAR,
        ];
        for zero_for_one in [true, false] {
            for start in Self::tick_array_starts_ahead(
                state,
                bitmap_extension,
                zero_for_one,
                tick_arrays_each_way,
            ) {
                let key = tick_array_pda(pool, start);
                if !keys.contains(&key) {
                    keys.push(key);
                }
            }
        }
        keys
    }

    /// Params and quote state for `pool` from accounts read together.
    pub fn from_quote_accounts(pool: Pubkey, accounts: &ClmmQuoteAccounts) -> Result<Self> {
        let state = PoolState::decode(accounts.pool)
            .map_err(|err| anyhow!("{pool} is not a Raydium CLMM pool: {err}"))?;
        let config = AmmConfig::decode(accounts.amm_config)
            .map_err(|err| anyhow!("CLMM AmmConfig {}: {err}", state.amm_config))?;
        let bitmap_extension = accounts
            .bitmap_extension
            .map(TickArrayBitmapExtension::decode)
            .transpose()
            .map_err(|err| anyhow!("CLMM bitmap extension of {pool}: {err}"))?;
        let tick_arrays = accounts
            .tick_arrays
            .iter()
            .map(|data| TickArrayState::decode(data))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|err| anyhow!("CLMM tick array of {pool}: {err}"))?;
        if tick_arrays.iter().any(|array| array.pool_id != pool) {
            return Err(anyhow!("A tick array given for {pool} belongs to another pool"));
        }
        if accounts.clock.len() < 40 {
            return Err(anyhow!("Clock sysvar data is too short"));
        }
        let epoch = u64::from_le_bytes(accounts.clock[16..24].try_into().unwrap());
        let unix_timestamp = i64::from_le_bytes(accounts.clock[32..40].try_into().unwrap()) as u64;
        let (token_0_program, token_0_data) = accounts.token_0_mint;
        let (token_1_program, token_1_data) = accounts.token_1_mint;
        let token_0_transfer_fee =
            token_transfer_fee_for_epoch(token_0_data, token_0_program, epoch)?;
        let token_1_transfer_fee =
            token_transfer_fee_for_epoch(token_1_data, token_1_program, epoch)?;
        Ok(Self {
            amm_config: state.amm_config,
            pool_state: pool,
            observation_state: state.observation_key,
            token_0_mint: state.token_mint_0,
            token_1_mint: state.token_mint_1,
            token_0_vault: state.token_vault_0,
            token_1_vault: state.token_vault_1,
            token_0_program,
            token_1_program,
            tick_arrays: tick_arrays
                .iter()
                .map(|array| tick_array_pda(&pool, array.start_tick_index))
                .collect(),
            tick_array_bitmap_extension: bitmap_extension
                .as_ref()
                .map(|_| tick_array_bitmap_extension(&pool)),
            sqrt_price_limit_x64: 0,
            spot: None,
            quote_state: Some(Box::new(ClmmQuoteState {
                pool: state,
                config,
                tick_arrays,
                bitmap_extension,
                token_0_transfer_fee,
                token_1_transfer_fee,
                unix_timestamp,
            })),
        })
    }

    /// Pool accounts and the state for exact quotes, in two RPC round trips:
    /// the pool, then what a quote reads per `quote_account_keys`.
    pub async fn from_pool_address_by_rpc(
        rpc: &SolanaRpcClient,
        pool: &Pubkey,
        input_mint: &Pubkey,
        output_mint: &Pubkey,
    ) -> Result<Self> {
        let bitmap = tick_array_bitmap_extension(pool);
        let first = rpc.get_multiple_accounts(&[*pool, bitmap]).await?;
        let pool_account = first[0]
            .as_ref()
            .filter(|account| account.owner == PROGRAM_ID)
            .ok_or_else(|| anyhow!("{pool} is not a Raydium CLMM pool"))?;
        let state = PoolState::decode(&pool_account.data)
            .map_err(|err| anyhow!("{pool} is not a Raydium CLMM pool: {err}"))?;
        let mints = [state.token_mint_0, state.token_mint_1];
        if !(mints.contains(input_mint) && mints.contains(output_mint) && input_mint != output_mint)
        {
            anyhow::bail!("CLMM swap mints do not match pool");
        }
        let extension = first[1]
            .as_ref()
            .map(|account| TickArrayBitmapExtension::decode(&account.data))
            .transpose()
            .map_err(|err| anyhow!("CLMM bitmap extension of {pool}: {err}"))?;
        let keys = Self::quote_account_keys(pool, &state, extension.as_ref(), QUOTE_TICK_ARRAYS);
        let accounts = rpc.get_multiple_accounts(&keys).await?;
        let data = |index: usize, what: &str| {
            accounts[index]
                .as_ref()
                .map(|account| account.data.as_slice())
                .ok_or_else(|| anyhow!("CLMM {what} of {pool} missing"))
        };
        let mint = |index: usize, what: &str| {
            accounts[index]
                .as_ref()
                .map(|account| (account.owner, account.data.as_slice()))
                .ok_or_else(|| anyhow!("CLMM {what} of {pool} missing"))
        };
        Self::from_quote_accounts(
            *pool,
            &ClmmQuoteAccounts {
                pool: data(0, "pool")?,
                amm_config: data(1, "AmmConfig")?,
                bitmap_extension: accounts[2].as_ref().map(|account| account.data.as_slice()),
                token_0_mint: mint(3, "token 0 mint")?,
                token_1_mint: mint(4, "token 1 mint")?,
                clock: data(5, "clock")?,
                tick_arrays: accounts[6..]
                    .iter()
                    .filter_map(|account| account.as_ref().map(|a| a.data.as_slice()))
                    .collect(),
            },
        )
    }
}
