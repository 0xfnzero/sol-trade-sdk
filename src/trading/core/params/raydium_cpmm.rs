use crate::common::SolanaRpcClient;
use solana_sdk::pubkey::Pubkey;
use spl_token_2022_interface::{
    extension::{
        transfer_fee::TransferFeeConfig, BaseStateWithExtensions, ExtensionType,
        StateWithExtensions,
    },
    state::Mint,
};

/// Active Token-2022 transfer fee for the epoch in which pool state was loaded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TokenTransferFee {
    pub basis_points: u16,
    pub maximum_fee: u64,
}

impl TokenTransferFee {
    #[inline]
    pub fn calculate(&self, amount: u64) -> u64 {
        if self.basis_points == 0 || amount == 0 {
            return 0;
        }
        let numerator = (amount as u128) * (self.basis_points as u128);
        numerator.div_ceil(10_000).min(self.maximum_fee as u128) as u64
    }

    /// Returns the fee that must be added so the recipient receives `post_fee_amount`.
    #[inline]
    pub fn calculate_inverse(&self, post_fee_amount: u64) -> u64 {
        if self.basis_points == 0 || post_fee_amount == 0 {
            return 0;
        }
        if self.basis_points >= 10_000 {
            return self.maximum_fee;
        }
        let numerator = (post_fee_amount as u128) * (self.basis_points as u128);
        numerator.div_ceil(10_000 - self.basis_points as u128).min(self.maximum_fee as u128) as u64
    }
}

#[cfg(test)]
mod transfer_fee_tests {
    use super::TokenTransferFee;

    #[test]
    fn transfer_fee_uses_ceiling_and_respects_maximum() {
        let fee = TokenTransferFee { basis_points: 300, maximum_fee: 2 };
        assert_eq!(fee.calculate(1), 1);
        assert_eq!(fee.calculate(100), 2);
    }

    #[test]
    fn zero_maximum_means_no_transfer_fee() {
        let fee = TokenTransferFee { basis_points: 300, maximum_fee: 0 };
        assert_eq!(fee.calculate(u64::MAX), 0);
        assert_eq!(fee.calculate_inverse(u64::MAX), 0);
    }

    #[test]
    fn inverse_transfer_fee_recovers_the_requested_post_fee_amount() {
        let fee = TokenTransferFee { basis_points: 300, maximum_fee: 1_000_000 };
        let post_fee_amount = 10_000;
        let inverse_fee = fee.calculate_inverse(post_fee_amount);
        let pre_fee_amount = post_fee_amount + inverse_fee;

        assert_eq!(pre_fee_amount - fee.calculate(pre_fee_amount), post_fee_amount);
    }
}

/// The transfer fee a mint charges in `epoch`: none for SPL Token mints, the
/// Token-2022 `TransferFeeConfig` for that epoch otherwise.
pub fn token_transfer_fee_for_epoch(
    data: &[u8],
    token_program: Pubkey,
    epoch: u64,
) -> Result<TokenTransferFee, anyhow::Error> {
    if token_program == crate::constants::TOKEN_PROGRAM {
        return Ok(TokenTransferFee::default());
    }
    let token_2022_program = Pubkey::new_from_array(spl_token_2022_interface::ID.to_bytes());
    if token_program != token_2022_program {
        return Err(anyhow::anyhow!("Unsupported CPMM token program: {}", token_program));
    }

    let mint = StateWithExtensions::<Mint>::unpack(data)
        .map_err(|error| anyhow::anyhow!("Failed to decode Token-2022 mint: {}", error))?;
    if !mint
        .get_extension_types()
        .map_err(|error| anyhow::anyhow!("Failed to inspect Token-2022 mint: {}", error))?
        .contains(&ExtensionType::TransferFeeConfig)
    {
        return Ok(TokenTransferFee::default());
    }
    let config = mint
        .get_extension::<TransferFeeConfig>()
        .map_err(|error| anyhow::anyhow!("Failed to decode Token-2022 transfer fee: {}", error))?;
    let fee = config.get_epoch_fee(epoch);
    Ok(TokenTransferFee {
        basis_points: fee.transfer_fee_basis_points.into(),
        maximum_fee: fee.maximum_fee.into(),
    })
}

/// RaydiumCpmm protocol specific parameters
/// Configuration parameters specific to Raydium CPMM trading protocol
#[derive(Clone)]
pub struct RaydiumCpmmParams {
    /// Pool address
    pub pool_state: Pubkey,
    /// Amm config address
    pub amm_config: Pubkey,
    /// Base token mint address
    pub base_mint: Pubkey,
    /// Quote token mint address
    pub quote_mint: Pubkey,
    /// Base token reserve amount in the pool
    pub base_reserve: u64,
    /// Quote token reserve amount in the pool
    pub quote_reserve: u64,
    /// Base token vault address
    pub base_vault: Pubkey,
    /// Quote token vault address
    pub quote_vault: Pubkey,
    /// Base token program ID
    pub base_token_program: Pubkey,
    /// Quote token program ID
    pub quote_token_program: Pubkey,
    /// Observation state account
    pub observation_state: Pubkey,
    /// Current fee rates loaded from the pool's AmmConfig.
    pub trade_fee_rate: u64,
    pub protocol_fee_rate: u64,
    pub fund_fee_rate: u64,
    pub creator_fee_rate: u64,
    /// Creator fee mode and enable flag loaded from PoolState.
    pub creator_fee_on: u8,
    pub enable_creator_fee: bool,
    /// Current epoch Token-2022 transfer fee configuration for token0/token1.
    pub base_transfer_fee: TokenTransferFee,
    pub quote_transfer_fee: TokenTransferFee,
}

impl RaydiumCpmmParams {
    pub fn from_trade(
        pool_state: Pubkey,
        amm_config: Pubkey,
        input_token_mint: Pubkey,
        output_token_mint: Pubkey,
        input_vault: Pubkey,
        output_vault: Pubkey,
        input_token_program: Pubkey,
        output_token_program: Pubkey,
        observation_state: Pubkey,
        base_reserve: u64,
        quote_reserve: u64,
    ) -> Self {
        Self {
            pool_state: pool_state,
            amm_config: amm_config,
            base_mint: input_token_mint,
            quote_mint: output_token_mint,
            base_reserve: base_reserve,
            quote_reserve: quote_reserve,
            base_vault: input_vault,
            quote_vault: output_vault,
            base_token_program: input_token_program,
            quote_token_program: output_token_program,
            observation_state: observation_state,
            trade_fee_rate: crate::instruction::utils::raydium_cpmm::accounts::TRADE_FEE_RATE,
            protocol_fee_rate: crate::instruction::utils::raydium_cpmm::accounts::PROTOCOL_FEE_RATE,
            fund_fee_rate: crate::instruction::utils::raydium_cpmm::accounts::FUND_FEE_RATE,
            creator_fee_rate: 0,
            creator_fee_on: 0,
            enable_creator_fee: false,
            base_transfer_fee: TokenTransferFee::default(),
            quote_transfer_fee: TokenTransferFee::default(),
        }
    }

    pub async fn from_pool_address_by_rpc(
        rpc: &SolanaRpcClient,
        pool_address: &Pubkey,
    ) -> Result<Self, anyhow::Error> {
        let pool =
            crate::instruction::utils::raydium_cpmm::fetch_pool_state(rpc, pool_address).await?;
        let amm_config =
            crate::instruction::utils::raydium_cpmm::fetch_amm_config(rpc, &pool.amm_config)
                .await?;
        let (token0_balance, token1_balance) =
            crate::instruction::utils::raydium_cpmm::get_pool_token_balances_from_vaults(
                rpc,
                &pool.token0_vault,
                &pool.token1_vault,
            )
            .await?;
        let token0_reserve = token0_balance
            .checked_sub(pool.protocol_fees_token0)
            .and_then(|amount| amount.checked_sub(pool.fund_fees_token0))
            .and_then(|amount| amount.checked_sub(pool.creator_fees_token0))
            .ok_or_else(|| anyhow::anyhow!("Raydium CPMM token0 fees exceed vault balance"))?;
        let token1_reserve = token1_balance
            .checked_sub(pool.protocol_fees_token1)
            .and_then(|amount| amount.checked_sub(pool.fund_fees_token1))
            .and_then(|amount| amount.checked_sub(pool.creator_fees_token1))
            .ok_or_else(|| anyhow::anyhow!("Raydium CPMM token1 fees exceed vault balance"))?;
        let token0_mint = rpc.get_account(&pool.token0_mint).await?;
        let token1_mint = rpc.get_account(&pool.token1_mint).await?;
        if token0_mint.owner != pool.token0_program || token1_mint.owner != pool.token1_program {
            return Err(anyhow::anyhow!(
                "Raydium CPMM mint owner does not match PoolState token program"
            ));
        }
        let epoch = rpc.get_epoch_info().await?.epoch;
        let token0_transfer_fee =
            token_transfer_fee_for_epoch(&token0_mint.data, pool.token0_program, epoch)?;
        let token1_transfer_fee =
            token_transfer_fee_for_epoch(&token1_mint.data, pool.token1_program, epoch)?;
        Ok(Self {
            pool_state: *pool_address,
            amm_config: pool.amm_config,
            base_mint: pool.token0_mint,
            quote_mint: pool.token1_mint,
            base_reserve: token0_reserve,
            quote_reserve: token1_reserve,
            base_vault: pool.token0_vault,
            quote_vault: pool.token1_vault,
            base_token_program: pool.token0_program,
            quote_token_program: pool.token1_program,
            observation_state: pool.observation_key,
            trade_fee_rate: amm_config.trade_fee_rate,
            protocol_fee_rate: amm_config.protocol_fee_rate,
            fund_fee_rate: amm_config.fund_fee_rate,
            creator_fee_rate: amm_config.creator_fee_rate,
            creator_fee_on: pool.creator_fee_on,
            enable_creator_fee: pool.enable_creator_fee,
            base_transfer_fee: token0_transfer_fee,
            quote_transfer_fee: token1_transfer_fee,
        })
    }
}
