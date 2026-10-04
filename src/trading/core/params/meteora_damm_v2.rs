use crate::common::SolanaRpcClient;
use crate::instruction::utils::meteora_damm_v2_types::Pool;
use crate::utils::calc::meteora_damm_v2::{fee_numerator, DammV2QuoteState};
use solana_sdk::pubkey::Pubkey;

/// MeteoraDammV2 protocol specific parameters
/// Configuration parameters specific to Meteora Damm V2 trading protocol
#[derive(Clone)]
pub struct MeteoraDammV2Params {
    pub pool: Pubkey,
    pub token_a_vault: Pubkey,
    pub token_b_vault: Pubkey,
    pub token_a_mint: Pubkey,
    pub token_b_mint: Pubkey,
    pub token_a_program: Pubkey,
    pub token_b_program: Pubkey,
    pub referral_token_account: Option<Pubkey>,
    /// `swap2` mode: 0 exact-in, 1 partial-fill (recommended default), 2 exact-out.
    pub swap_mode: u8,
    /// Include the instructions sysvar remaining account when the pool's rate limiter applies.
    pub include_rate_limiter_sysvar: bool,
    /// The pool state to quote the minimum output from. Without it a swap
    /// needs `fixed_output_amount`.
    pub quote: Option<DammV2QuoteState>,
}

impl MeteoraDammV2Params {
    pub fn new(
        pool: Pubkey,
        token_a_vault: Pubkey,
        token_b_vault: Pubkey,
        token_a_mint: Pubkey,
        token_b_mint: Pubkey,
        token_a_program: Pubkey,
        token_b_program: Pubkey,
    ) -> Self {
        Self {
            pool,
            token_a_vault,
            token_b_vault,
            token_a_mint,
            token_b_mint,
            token_a_program,
            token_b_program,
            referral_token_account: None,
            swap_mode: crate::instruction::utils::meteora_damm_v2::SWAP_MODE_PARTIAL_FILL,
            include_rate_limiter_sysvar: false,
            quote: None,
        }
    }

    pub fn with_quote(mut self, quote: DammV2QuoteState) -> Self {
        self.quote = Some(quote);
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

    pub fn with_rate_limiter_sysvar(mut self, include: bool) -> Self {
        self.include_rate_limiter_sysvar = include;
        self
    }

    pub async fn from_pool_address_by_rpc(
        rpc: &SolanaRpcClient,
        pool_address: &Pubkey,
    ) -> Result<Self, anyhow::Error> {
        let pool_data =
            crate::instruction::utils::meteora_damm_v2::fetch_pool(rpc, pool_address).await?;
        let mint_accounts =
            rpc.get_multiple_accounts(&[pool_data.token_a_mint, pool_data.token_b_mint]).await?;
        let token_a_program = mint_accounts
            .get(0)
            .and_then(|a| a.as_ref())
            .map(|a| a.owner)
            .ok_or_else(|| anyhow::anyhow!("Token A mint account not found"))?;
        let token_b_program = mint_accounts
            .get(1)
            .and_then(|a| a.as_ref())
            .map(|a| a.owner)
            .ok_or_else(|| anyhow::anyhow!("Token B mint account not found"))?;
        // The pool counts time in slots (activation type 0) or seconds; only a
        // fee still on its schedule needs the slot.
        let slot = if pool_data.activation_type == 0
            && pool_data.pool_fees.base_fee.period_frequency != 0
        {
            rpc.get_slot().await?
        } else {
            0
        };
        Ok(Self::from_pool_state(pool_address, &pool_data, token_a_program, token_b_program, slot))
    }

    /// The parameters of a pool account already read, whose mints' token
    /// programs are known; `slot` is the current one, which a pool counting
    /// time in slots prices its scheduled fee by.
    pub fn from_pool_state(
        pool_address: &Pubkey,
        pool: &Pool,
        token_a_program: Pubkey,
        token_b_program: Pubkey,
        slot: u64,
    ) -> Self {
        let point = if pool.activation_type == 0 {
            slot
        } else {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |time| time.as_secs())
        };
        Self::new(
            *pool_address,
            pool.token_a_vault,
            pool.token_b_vault,
            pool.token_a_mint,
            pool.token_b_mint,
            token_a_program,
            token_b_program,
        )
        .with_quote(DammV2QuoteState::from_pool(pool, fee_numerator(pool, point)))
    }
}
