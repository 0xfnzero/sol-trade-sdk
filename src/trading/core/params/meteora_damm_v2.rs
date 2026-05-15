use crate::common::SolanaRpcClient;
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
        }
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
        Ok(Self {
            pool: *pool_address,
            token_a_vault: pool_data.token_a_vault,
            token_b_vault: pool_data.token_b_vault,
            token_a_mint: pool_data.token_a_mint,
            token_b_mint: pool_data.token_b_mint,
            token_a_program,
            token_b_program,
            referral_token_account: None,
            swap_mode: crate::instruction::utils::meteora_damm_v2::SWAP_MODE_PARTIAL_FILL,
            include_rate_limiter_sysvar: false,
        })
    }
}
