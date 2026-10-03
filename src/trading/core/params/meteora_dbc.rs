use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use solana_sdk::{instruction::AccountMeta, pubkey::Pubkey};

use crate::common::SolanaRpcClient;
use crate::instruction::utils::meteora_dbc::{
    fetch_config_cached, fetch_extra_account_metas, fetch_pool, ExtraAccountMeta,
    SWAP_MODE_PARTIAL_FILL,
};
use crate::instruction::utils::meteora_dbc_types::{DbcConfig, DbcPool, ACTIVATION_SLOT};
use crate::utils::calc::meteora_dbc::{
    current_point, fee_numerator, quote_exact_in, rate_limiter_applies, rate_limiter_fee_numerator,
    DbcQuote, MAX_FEE_NUMERATOR,
};

/// Where a transfer-hook pool's swap takes the base token's hook accounts from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DbcTransferHook {
    /// The accounts another swap of the pool passed. Right for every wallet
    /// when the hook's accounts hang off the mint alone, as those of the
    /// mainnet hooks seen so far do.
    Accounts(Vec<AccountMeta>),
    /// The hook's extra account list, resolved for each swap's own accounts.
    Metas { program: Pubkey, metas: Vec<ExtraAccountMeta> },
}

/// The curve a swap's output is quoted on.
#[derive(Clone, Debug)]
pub struct DbcQuoteState {
    pub config: Arc<DbcConfig>,
    pub sqrt_price: u128,
    /// The fee numerator a swap pays now.
    pub fee_numerator: u64,
    /// The config's rate limiter prices buys now, by their size, on top of
    /// `fee_numerator`.
    pub rate_limited_buys: bool,
}

impl DbcQuoteState {
    /// The fee numerator of a swap of `amount_in`.
    pub fn fee_numerator_for(&self, is_buy: bool, amount_in: u64) -> u64 {
        if is_buy && self.rate_limited_buys {
            if let Some(limited) = rate_limiter_fee_numerator(&self.config, amount_in) {
                // `fee_numerator` is the cliff fee plus the dynamic fee.
                let dynamic =
                    self.fee_numerator.saturating_sub(self.config.base_fee.cliff_fee_numerator);
                return limited.saturating_add(dynamic).min(MAX_FEE_NUMERATOR);
            }
        }
        self.fee_numerator
    }

    /// What `amount_in` of quote (a buy) or of base buys, as a partial fill
    /// pays it.
    pub fn quote_exact_in(&self, is_buy: bool, amount_in: u64) -> anyhow::Result<DbcQuote> {
        let fee_numerator = self.fee_numerator_for(is_buy, amount_in);
        quote_exact_in(&self.config, self.sqrt_price, fee_numerator, is_buy, amount_in)
    }
}

/// Meteora Dynamic Bonding Curve pool parameters.
#[derive(Clone, Debug)]
pub struct MeteoraDbcParams {
    pub pool: Pubkey,
    pub config: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub base_vault: Pubkey,
    pub quote_vault: Pubkey,
    pub base_token_program: Pubkey,
    pub quote_token_program: Pubkey,
    /// Set for a transfer-hook pool, which trades with
    /// `swap2_with_transfer_hook`.
    pub transfer_hook: Option<DbcTransferHook>,
    /// Pass the instructions sysvar, as a buy priced by the rate limiter must.
    pub include_instructions_sysvar: bool,
    pub referral_token_account: Option<Pubkey>,
    /// `swap2` mode: 0 exact in, 1 partial fill (the default), 2 exact out.
    pub swap_mode: u8,
    /// The curve to quote the minimum output on. Without it a swap needs
    /// `fixed_output_amount`.
    pub quote: Option<DbcQuoteState>,
}

impl MeteoraDbcParams {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pool: Pubkey,
        config: Pubkey,
        base_mint: Pubkey,
        quote_mint: Pubkey,
        base_vault: Pubkey,
        quote_vault: Pubkey,
        base_token_program: Pubkey,
        quote_token_program: Pubkey,
    ) -> Self {
        Self {
            pool,
            config,
            base_mint,
            quote_mint,
            base_vault,
            quote_vault,
            base_token_program,
            quote_token_program,
            transfer_hook: None,
            include_instructions_sysvar: false,
            referral_token_account: None,
            swap_mode: SWAP_MODE_PARTIAL_FILL,
            quote: None,
        }
    }

    pub fn with_transfer_hook(mut self, transfer_hook: DbcTransferHook) -> Self {
        self.transfer_hook = Some(transfer_hook);
        self
    }

    pub fn with_instructions_sysvar(mut self, include: bool) -> Self {
        self.include_instructions_sysvar = include;
        self
    }

    pub fn with_referral_token_account(mut self, referral_token_account: Pubkey) -> Self {
        self.referral_token_account = Some(referral_token_account);
        self
    }

    pub fn with_swap_mode(mut self, swap_mode: u8) -> Self {
        self.swap_mode = swap_mode;
        self
    }

    pub fn with_quote(mut self, quote: DbcQuoteState) -> Self {
        self.quote = Some(quote);
        self
    }

    /// Loads a pool with its config, token programs, current fee and, for a
    /// transfer-hook pool, the hook's extra account list.
    pub async fn from_pool_address_by_rpc(
        rpc: &SolanaRpcClient,
        pool_address: &Pubkey,
    ) -> anyhow::Result<Self> {
        let pool = fetch_pool(rpc, pool_address).await?;
        Self::from_pool_by_rpc(rpc, pool_address, &pool).await
    }

    /// [`Self::from_pool_address_by_rpc`] for a pool account already read.
    pub async fn from_pool_by_rpc(
        rpc: &SolanaRpcClient,
        pool_address: &Pubkey,
        pool: &DbcPool,
    ) -> anyhow::Result<Self> {
        let config = fetch_config_cached(rpc, &pool.config).await?;
        let mints = rpc.get_multiple_accounts(&[pool.base_mint, config.quote_mint]).await?;
        let owner = |index: usize, mint: &Pubkey| {
            mints
                .get(index)
                .and_then(|account| account.as_ref())
                .map(|account| account.owner)
                .ok_or_else(|| anyhow::anyhow!("Mint account {mint} not found"))
        };
        let base_token_program = owner(0, &pool.base_mint)?;
        let quote_token_program = owner(1, &config.quote_mint)?;

        let transfer_hook = match (pool.transfer_hook, config.transfer_hook_program) {
            (false, _) => None,
            (true, Some(program)) => Some(DbcTransferHook::Metas {
                program,
                metas: fetch_extra_account_metas(rpc, &pool.base_mint, &program).await?,
            }),
            (true, None) => {
                return Err(anyhow::anyhow!(
                    "Transfer-hook pool {pool_address} has a config without a hook program"
                ))
            }
        };

        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |time| time.as_secs());
        // Only fees that change with time need the slot.
        let slot =
            if config.activation_type == ACTIVATION_SLOT && config.base_fee.second_factor != 0 {
                rpc.get_slot().await?
            } else {
                0
            };
        let point = current_point(&config, slot, now);
        // Priced as a sale: a rate-limited buy pays more, by its size.
        let fee_numerator = fee_numerator(
            &config,
            pool.activation_point,
            pool.volatility.volatility_accumulator,
            point,
            false,
            0,
        )?;
        let rate_limited_buys = rate_limiter_applies(&config, pool.activation_point, point, true);
        Ok(Self {
            pool: *pool_address,
            config: pool.config,
            base_mint: pool.base_mint,
            quote_mint: config.quote_mint,
            base_vault: pool.base_vault,
            quote_vault: pool.quote_vault,
            base_token_program,
            quote_token_program,
            transfer_hook,
            include_instructions_sysvar: rate_limited_buys,
            referral_token_account: None,
            swap_mode: SWAP_MODE_PARTIAL_FILL,
            quote: Some(DbcQuoteState {
                config,
                sqrt_price: pool.sqrt_price,
                fee_numerator,
                rate_limited_buys,
            }),
        })
    }
}
