use crate::common::SolanaRpcClient;
use crate::trading::core::params::raydium_cpmm::{token_transfer_fee_for_epoch, TokenTransferFee};
use solana_sdk::pubkey::Pubkey;

/// Parameters for the shared LaunchLab program.
#[derive(Clone)]
pub struct BonkParams {
    pub virtual_base: u128,
    pub virtual_quote: u128,
    pub real_base: u128,
    pub real_quote: u128,
    /// Maximum base amount sold by the curve. Zero means unavailable for legacy callers.
    pub total_base_sell: u128,
    pub pool_state: Pubkey,
    pub base_vault: Pubkey,
    pub quote_vault: Pubkey,
    /// Token program ID
    pub mint_token_program: Pubkey,
    /// Quote mint. A zero pubkey preserves the legacy SOL/USD1 inference.
    pub quote_mint: Pubkey,
    /// Quote token program. A zero pubkey defaults to the classic SPL Token program.
    pub quote_token_program: Pubkey,
    pub platform_config: Pubkey,
    pub platform_associated_account: Pubkey,
    pub creator_associated_account: Pubkey,
    pub global_config: Pubkey,
    /// Current LaunchLab fee configuration. Rates use the on-chain 1e6 denominator.
    pub curve_type: u8,
    pub trade_fee_rate: u64,
    pub platform_fee_rate: u64,
    pub creator_fee_rate: u64,
    /// Current epoch Token-2022 transfer fees for the base and quote mints.
    pub base_transfer_fee: TokenTransferFee,
    pub quote_transfer_fee: TokenTransferFee,
}

impl Default for BonkParams {
    fn default() -> Self {
        Self {
            virtual_base: 0,
            virtual_quote: 0,
            real_base: 0,
            real_quote: 0,
            total_base_sell: 0,
            pool_state: Pubkey::default(),
            base_vault: Pubkey::default(),
            quote_vault: Pubkey::default(),
            mint_token_program: Pubkey::default(),
            quote_mint: Pubkey::default(),
            quote_token_program: Pubkey::default(),
            platform_config: Pubkey::default(),
            platform_associated_account: Pubkey::default(),
            creator_associated_account: Pubkey::default(),
            global_config: Pubkey::default(),
            curve_type: 0,
            trade_fee_rate: 2_500,
            platform_fee_rate: 10_000,
            creator_fee_rate: 0,
            base_transfer_fee: TokenTransferFee::default(),
            quote_transfer_fee: TokenTransferFee::default(),
        }
    }
}

impl BonkParams {
    /// Legacy event constructor: configured fees/total sell cap are still defaults.
    /// Use SubscriptionAccountCache::launchlab for complete current parameters.
    pub fn from_launchlab_trade(
        virtual_base: u64,
        virtual_quote: u64,
        real_base_after: u64,
        real_quote_after: u64,
        pool_state: Pubkey,
        base_vault: Pubkey,
        quote_vault: Pubkey,
        base_token_program: Pubkey,
        quote_mint: Pubkey,
        quote_token_program: Pubkey,
        platform_config: Pubkey,
        platform_associated_account: Pubkey,
        creator_associated_account: Pubkey,
        global_config: Pubkey,
    ) -> Self {
        Self {
            virtual_base: virtual_base as u128,
            virtual_quote: virtual_quote as u128,
            real_base: real_base_after as u128,
            real_quote: real_quote_after as u128,
            pool_state,
            base_vault,
            quote_vault,
            mint_token_program: base_token_program,
            quote_mint,
            quote_token_program,
            platform_config,
            platform_associated_account,
            creator_associated_account,
            global_config,
            ..Default::default()
        }
    }

    pub fn immediate_sell(
        mint_token_program: Pubkey,
        platform_config: Pubkey,
        platform_associated_account: Pubkey,
        creator_associated_account: Pubkey,
        global_config: Pubkey,
    ) -> Self {
        Self {
            mint_token_program,
            platform_config,
            platform_associated_account,
            creator_associated_account,
            global_config,
            ..Default::default()
        }
    }
    /// Legacy partial constructor; quote and current fees are unavailable here.
    /// Prefer SubscriptionAccountCache::launchlab for stock-quoted copy trades.
    pub fn from_trade(
        virtual_base: u64,
        virtual_quote: u64,
        real_base_after: u64,
        real_quote_after: u64,
        pool_state: Pubkey,
        base_vault: Pubkey,
        quote_vault: Pubkey,
        base_token_program: Pubkey,
        platform_config: Pubkey,
        platform_associated_account: Pubkey,
        creator_associated_account: Pubkey,
        global_config: Pubkey,
    ) -> Self {
        Self {
            virtual_base: virtual_base as u128,
            virtual_quote: virtual_quote as u128,
            real_base: real_base_after as u128,
            real_quote: real_quote_after as u128,
            pool_state: pool_state,
            base_vault: base_vault,
            quote_vault: quote_vault,
            mint_token_program: base_token_program,
            platform_config: platform_config,
            platform_associated_account: platform_associated_account,
            creator_associated_account: creator_associated_account,
            global_config: global_config,
            quote_mint: Pubkey::default(),
            quote_token_program: Pubkey::default(),
            ..Default::default()
        }
    }

    pub fn from_dev_trade(
        is_exact_in: bool,
        amount_in: u64,
        amount_out: u64,
        pool_state: Pubkey,
        base_vault: Pubkey,
        quote_vault: Pubkey,
        base_token_program: Pubkey,
        platform_config: Pubkey,
        platform_associated_account: Pubkey,
        creator_associated_account: Pubkey,
        global_config: Pubkey,
    ) -> Self {
        const DEFAULT_VIRTUAL_BASE: u128 = 1073025605596382;
        const DEFAULT_VIRTUAL_QUOTE: u128 = 30000852951;
        let _amount_in = if is_exact_in {
            amount_in
        } else {
            crate::instruction::utils::bonk::get_amount_in(
                amount_out,
                crate::instruction::utils::bonk::accounts::PROTOCOL_FEE_RATE,
                crate::instruction::utils::bonk::accounts::PLATFORM_FEE_RATE,
                crate::instruction::utils::bonk::accounts::SHARE_FEE_RATE,
                DEFAULT_VIRTUAL_BASE,
                DEFAULT_VIRTUAL_QUOTE,
                0,
                0,
                0,
            )
        };
        let real_quote = crate::instruction::utils::bonk::get_amount_in_net(
            amount_in,
            crate::instruction::utils::bonk::accounts::PROTOCOL_FEE_RATE,
            crate::instruction::utils::bonk::accounts::PLATFORM_FEE_RATE,
            crate::instruction::utils::bonk::accounts::SHARE_FEE_RATE,
        ) as u128;
        let _amount_out = if is_exact_in {
            crate::instruction::utils::bonk::get_amount_out(
                amount_in,
                crate::instruction::utils::bonk::accounts::PROTOCOL_FEE_RATE,
                crate::instruction::utils::bonk::accounts::PLATFORM_FEE_RATE,
                crate::instruction::utils::bonk::accounts::SHARE_FEE_RATE,
                DEFAULT_VIRTUAL_BASE,
                DEFAULT_VIRTUAL_QUOTE,
                0,
                0,
                0,
            ) as u128
        } else {
            amount_out as u128
        };
        let real_base = _amount_out;
        Self {
            virtual_base: DEFAULT_VIRTUAL_BASE,
            virtual_quote: DEFAULT_VIRTUAL_QUOTE,
            real_base: real_base,
            real_quote: real_quote,
            pool_state: pool_state,
            base_vault: base_vault,
            quote_vault: quote_vault,
            mint_token_program: base_token_program,
            platform_config: platform_config,
            platform_associated_account: platform_associated_account,
            creator_associated_account: creator_associated_account,
            global_config: global_config,
            quote_mint: Pubkey::default(),
            quote_token_program: Pubkey::default(),
            ..Default::default()
        }
    }

    pub async fn from_mint_by_rpc(
        rpc: &SolanaRpcClient,
        mint: &Pubkey,
        usd1_pool: bool,
    ) -> Result<Self, anyhow::Error> {
        let pool_address = crate::instruction::utils::bonk::get_pool_pda(
            mint,
            if usd1_pool {
                &crate::constants::USD1_TOKEN_ACCOUNT
            } else {
                &crate::constants::WSOL_TOKEN_ACCOUNT
            },
        )
        .ok_or_else(|| anyhow::anyhow!("Failed to derive LaunchLab pool address"))?;
        Self::from_pool_by_rpc(rpc, &pool_address).await
    }

    /// Loads any LaunchLab pool, including StonkFun pools with non-SOL quotes.
    pub async fn from_pool_by_rpc(
        rpc: &SolanaRpcClient,
        pool_address: &Pubkey,
    ) -> Result<Self, anyhow::Error> {
        let pool_data =
            crate::instruction::utils::bonk::fetch_pool_state(rpc, pool_address).await?;
        let base_mint_account = rpc.get_account(&pool_data.base_mint).await?;
        let quote_mint_account = rpc.get_account(&pool_data.quote_mint).await?;
        let fee_config = crate::instruction::utils::bonk::fetch_fee_config(
            rpc,
            &pool_data.global_config,
            &pool_data.platform_config,
        )
        .await?;
        if fee_config.curve_type != 0 {
            return Err(anyhow::anyhow!(
                "Unsupported LaunchLab curve type: {}",
                fee_config.curve_type
            ));
        }
        let epoch = rpc.get_epoch_info().await?.epoch;
        let base_transfer_fee =
            token_transfer_fee_for_epoch(&base_mint_account.data, base_mint_account.owner, epoch)?;
        let quote_transfer_fee = token_transfer_fee_for_epoch(
            &quote_mint_account.data,
            quote_mint_account.owner,
            epoch,
        )?;
        let platform_associated_account =
            crate::instruction::utils::bonk::get_platform_associated_account_for_quote(
                &pool_data.platform_config,
                &pool_data.quote_mint,
            );
        let creator_associated_account =
            crate::instruction::utils::bonk::get_creator_associated_account_for_quote(
                &pool_data.creator,
                &pool_data.quote_mint,
            );
        let platform_associated_account = platform_associated_account
            .ok_or_else(|| anyhow::anyhow!("Failed to derive LaunchLab platform account"))?;
        let creator_associated_account = creator_associated_account
            .ok_or_else(|| anyhow::anyhow!("Failed to derive LaunchLab creator account"))?;
        Ok(Self {
            virtual_base: pool_data.virtual_base as u128,
            virtual_quote: pool_data.virtual_quote as u128,
            real_base: pool_data.real_base as u128,
            real_quote: pool_data.real_quote as u128,
            total_base_sell: pool_data.total_base_sell as u128,
            pool_state: *pool_address,
            base_vault: pool_data.base_vault,
            quote_vault: pool_data.quote_vault,
            mint_token_program: base_mint_account.owner,
            quote_mint: pool_data.quote_mint,
            quote_token_program: quote_mint_account.owner,
            platform_config: pool_data.platform_config,
            platform_associated_account,
            creator_associated_account,
            global_config: pool_data.global_config,
            curve_type: fee_config.curve_type,
            trade_fee_rate: fee_config.trade_fee_rate,
            platform_fee_rate: fee_config.platform_fee_rate,
            creator_fee_rate: fee_config.creator_fee_rate,
            base_transfer_fee,
            quote_transfer_fee,
        })
    }

    /// Derives and loads a LaunchLab pool from its base and quote mints.
    pub async fn from_mints_by_rpc(
        rpc: &SolanaRpcClient,
        base_mint: &Pubkey,
        quote_mint: &Pubkey,
    ) -> Result<Self, anyhow::Error> {
        let pool_address = crate::instruction::utils::bonk::get_pool_pda(base_mint, quote_mint)
            .ok_or_else(|| anyhow::anyhow!("Failed to derive LaunchLab pool address"))?;
        Self::from_pool_by_rpc(rpc, &pool_address).await
    }
}

/// StonkFun uses LaunchLab with platform-specific configuration accounts.
pub type StonkFunParams = BonkParams;
/// Preferred generic name for shared LaunchLab parameters.
pub type LaunchLabParams = BonkParams;
