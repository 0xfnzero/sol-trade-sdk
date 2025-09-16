pub mod common;
pub mod constants;
pub mod instruction;
pub mod protos;
pub mod swqos;
pub mod trading;
pub mod utils;
use crate::constants::trade::trade::DEFAULT_SLIPPAGE;
use crate::swqos::settings::SwqosSettings;
use crate::trading::core::params::BonkParams;
use crate::trading::core::params::PumpFunParams;
use crate::trading::core::params::PumpSwapParams;
use crate::trading::core::params::RaydiumAmmV4Params;
use crate::trading::core::params::RaydiumCpmmParams;
use crate::trading::core::traits::ProtocolParams;
use crate::trading::factory::DexType;
use crate::trading::InternalBuyParams;
use crate::trading::InternalSellParams;
use crate::trading::MiddlewareManager;
use crate::trading::TradeFactory;
use common::SolanaRpcClient;
use parking_lot::Mutex;
use rustls::crypto::{ring::default_provider, CryptoProvider};
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::hash::Hash;
use solana_sdk::signer::Signer;
use solana_sdk::{pubkey::Pubkey, signature::Keypair, signature::Signature};
pub use solana_streamer_sdk;
use std::sync::Arc;

/// Main trading client for Solana DeFi protocols
///
/// `SolanaTrade` provides a unified interface for trading across multiple Solana DEXs
/// including PumpFun, PumpSwap, Bonk, Raydium AMM V4, and Raydium CPMM.
/// It manages RPC connections, transaction signing, and SWQOS (Solana Web Quality of Service) settings.
pub struct SolanaTrade {
    /// The keypair used for signing all transactions
    pub payer: Arc<Keypair>,
    /// RPC client for blockchain interactions
    pub rpc: Arc<SolanaRpcClient>,
    /// SWQOS settings for transaction priority and routing
    pub swqos_settings: Vec<Arc<SwqosSettings>>,
    /// Optional middleware manager for custom transaction processing
    pub middleware_manager: Option<Arc<MiddlewareManager>>,
}

static INSTANCE: Mutex<Option<Arc<SolanaTrade>>> = Mutex::new(None);

impl Clone for SolanaTrade {
    fn clone(&self) -> Self {
        Self {
            payer: self.payer.clone(),
            rpc: self.rpc.clone(),
            swqos_settings: self.swqos_settings.clone(),
            middleware_manager: self.middleware_manager.clone(),
        }
    }
}

/// Parameters for executing buy orders across different DEX protocols
///
/// Contains all necessary configuration for purchasing tokens, including
/// protocol-specific settings, account management options, and transaction preferences.
#[derive(Clone)]
pub struct TradeBuyParams {
    // Trading configuration
    /// The DEX protocol to use for the trade
    pub dex_type: DexType,
    /// Public key of the token to purchase
    pub mint: Pubkey,
    /// Amount of SOL to spend (in lamports)
    pub sol_amount: u64,
    /// Optional slippage tolerance in basis points (e.g., 100 = 1%)
    pub slippage_basis_points: Option<u64>,
    /// Recent blockhash for transaction validity
    pub recent_blockhash: Hash,
    /// Protocol-specific parameters (PumpFun, Raydium, etc.)
    pub extension_params: Box<dyn ProtocolParams>,
    // Extended configuration
    /// Optional custom compute unit limit for the transaction
    pub custom_cu_limit: Option<u32>,
    /// Optional address lookup table for transaction size optimization
    pub lookup_table_key: Option<Pubkey>,
    /// Whether to wait for transaction confirmation before returning
    pub wait_transaction_confirmed: bool,
    /// Whether to create wrapped SOL associated token account
    pub create_wsol_ata: bool,
    /// Whether to close wrapped SOL associated token account after trade
    pub close_wsol_ata: bool,
    /// Whether to create token mint associated token account
    pub create_mint_ata: bool,
    /// Whether to enable seed-based optimization for account creation
    pub open_seed_optimize: bool,
}

/// Parameters for executing sell orders across different DEX protocols
///
/// Contains all necessary configuration for selling tokens, including
/// protocol-specific settings, tip preferences, account management options, and transaction preferences.
#[derive(Clone)]
pub struct TradeSellParams {
    // Trading configuration
    /// The DEX protocol to use for the trade
    pub dex_type: DexType,
    /// Public key of the token to sell
    pub mint: Pubkey,
    /// Amount of tokens to sell (in smallest token units)
    pub token_amount: u64,
    /// Optional slippage tolerance in basis points (e.g., 100 = 1%)
    pub slippage_basis_points: Option<u64>,
    /// Recent blockhash for transaction validity
    pub recent_blockhash: Hash,
    /// Whether to include tip for transaction priority
    pub with_tip: bool,
    /// Protocol-specific parameters (PumpFun, Raydium, etc.)
    pub extension_params: Box<dyn ProtocolParams>,
    // Extended configuration
    /// Optional custom compute unit limit for the transaction
    pub custom_cu_limit: Option<u32>,
    /// Optional address lookup table for transaction size optimization
    pub lookup_table_key: Option<Pubkey>,
    /// Whether to wait for transaction confirmation before returning
    pub wait_transaction_confirmed: bool,
    /// Whether to create wrapped SOL associated token account
    pub create_wsol_ata: bool,
    /// Whether to close wrapped SOL associated token account after trade
    pub close_wsol_ata: bool,
    /// Whether to enable seed-based optimization for account creation
    pub open_seed_optimize: bool,
}

impl SolanaTrade {
    /// Creates a new SolanaTrade instance with the specified configuration
    ///
    /// This function initializes the trading system with RPC connection, SWQOS settings,
    /// and sets up necessary components for trading operations.
    ///
    /// # Arguments
    /// * `payer` - The keypair used for signing transactions
    /// * `rpc_url` - Solana RPC endpoint URL
    /// * `commitment` - Transaction commitment level for RPC calls
    /// * `swqos_settings` - List of SWQOS (Solana Web Quality of Service) configurations
    ///
    /// # Returns
    /// Returns a configured `SolanaTrade` instance ready for trading operations
    #[inline]
    pub async fn new(
        payer: Arc<Keypair>,
        rpc_url: String,
        commitment: CommitmentConfig,
        mut swqos_settings: Vec<SwqosSettings>,
    ) -> Self {
        crate::common::fast_fn::fast_init(&payer.try_pubkey().unwrap());

        if CryptoProvider::get_default().is_none() {
            let _ = default_provider()
                .install_default()
                .map_err(|e| anyhow::anyhow!("Failed to install crypto provider: {:?}", e));
        }

        let rpc_url = rpc_url.clone();
        let commitment = commitment.clone();

        for swqos in &mut swqos_settings {
            swqos.setup_swqos_client(rpc_url.clone(), commitment.clone());
        }

        let rpc =
            Arc::new(SolanaRpcClient::new_with_commitment(rpc_url.clone(), commitment.clone()));
        common::seed::update_rents(&rpc).await.unwrap();
        common::seed::start_rent_updater(rpc.clone());

        let instance = Self {
            payer,
            rpc,
            swqos_settings: swqos_settings.into_iter().map(|s| Arc::new(s)).collect(),
            middleware_manager: None,
        };

        let mut current = INSTANCE.lock();
        *current = Some(Arc::new(instance.clone()));

        instance
    }

    /// Adds a middleware manager to the SolanaTrade instance
    ///
    /// Middleware managers can be used to implement custom logic that runs before or after trading operations,
    /// such as logging, monitoring, or custom validation.
    ///
    /// # Arguments
    /// * `middleware_manager` - The middleware manager to attach
    ///
    /// # Returns
    /// Returns the modified SolanaTrade instance with middleware manager attached
    pub fn with_middleware_manager(mut self, middleware_manager: MiddlewareManager) -> Self {
        self.middleware_manager = Some(Arc::new(middleware_manager));
        self
    }

    /// Gets the RPC client instance for direct Solana blockchain interactions
    ///
    /// This provides access to the underlying Solana RPC client that can be used
    /// for custom blockchain operations outside of the trading framework.
    ///
    /// # Returns
    /// Returns a reference to the Arc-wrapped SolanaRpcClient instance
    pub fn get_rpc(&self) -> &Arc<SolanaRpcClient> {
        &self.rpc
    }

    /// Gets the current globally shared SolanaTrade instance
    ///
    /// This provides access to the singleton instance that was created with `new()`.
    /// Useful for accessing the trading instance from different parts of the application.
    ///
    /// # Returns
    /// Returns the Arc-wrapped SolanaTrade instance
    ///
    /// # Panics
    /// Panics if no instance has been initialized yet. Make sure to call `new()` first.
    pub fn get_instance() -> Arc<Self> {
        let instance = INSTANCE.lock();
        instance
            .as_ref()
            .expect("SolanaTrade instance not initialized. Please call new() first.")
            .clone()
    }

    /// Execute a buy order for a specified token
    ///
    /// # Arguments
    ///
    /// * `params` - Buy trade parameters containing all necessary trading configuration
    ///
    /// # Returns
    ///
    /// Returns `Ok(Signature)` with the transaction signature if the buy order is successfully executed,
    /// or an error if the transaction fails.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - Invalid protocol parameters are provided for the specified DEX type
    /// - The transaction fails to execute
    /// - Network or RPC errors occur
    /// - Insufficient SOL balance for the purchase
    /// - Required accounts cannot be created or accessed
    pub async fn buy(&self, params: TradeBuyParams) -> Result<Signature, anyhow::Error> {
        if params.slippage_basis_points.is_none() {
            println!(
                "slippage_basis_points is none, use default slippage basis points: {}",
                DEFAULT_SLIPPAGE
            );
        }
        let executor = TradeFactory::create_executor(params.dex_type.clone());
        let protocol_params = params.extension_params;

        let buy_params = InternalBuyParams {
            rpc: Some(self.rpc.clone()),
            payer: self.payer.clone(),
            mint: params.mint,
            sol_amount: params.sol_amount,
            slippage_basis_points: params.slippage_basis_points,
            lookup_table_key: params.lookup_table_key,
            recent_blockhash: params.recent_blockhash,
            data_size_limit: 256 * 1024,
            wait_transaction_confirmed: params.wait_transaction_confirmed,
            protocol_params: protocol_params.clone(),
            open_seed_optimize: params.open_seed_optimize,
            create_wsol_ata: params.create_wsol_ata,
            close_wsol_ata: params.close_wsol_ata,
            create_mint_ata: params.create_mint_ata,
            swqos_settings: self.swqos_settings.clone(),
            middleware_manager: self.middleware_manager.clone(),
            custom_cu_limit: params.custom_cu_limit,
        };

        // Validate protocol params
        let is_valid_params = match params.dex_type {
            DexType::PumpFun => protocol_params.as_any().downcast_ref::<PumpFunParams>().is_some(),
            DexType::PumpSwap => {
                protocol_params.as_any().downcast_ref::<PumpSwapParams>().is_some()
            }
            DexType::Bonk => protocol_params.as_any().downcast_ref::<BonkParams>().is_some(),
            DexType::RaydiumCpmm => {
                protocol_params.as_any().downcast_ref::<RaydiumCpmmParams>().is_some()
            }
            DexType::RaydiumAmmV4 => {
                protocol_params.as_any().downcast_ref::<RaydiumAmmV4Params>().is_some()
            }
        };

        if !is_valid_params {
            return Err(anyhow::anyhow!("Invalid protocol params for Trade"));
        }

        executor.buy_with_tip(buy_params).await
    }

    /// Execute a sell order for a specified token
    ///
    /// # Arguments
    ///
    /// * `params` - Sell trade parameters containing all necessary trading configuration
    ///
    /// # Returns
    ///
    /// Returns `Ok(Signature)` with the transaction signature if the sell order is successfully executed,
    /// or an error if the transaction fails.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - Invalid protocol parameters are provided for the specified DEX type
    /// - The transaction fails to execute
    /// - Network or RPC errors occur
    /// - Insufficient token balance for the sale
    /// - Token account doesn't exist or is not properly initialized
    /// - Required accounts cannot be created or accessed
    pub async fn sell(&self, params: TradeSellParams) -> Result<Signature, anyhow::Error> {
        if params.slippage_basis_points.is_none() {
            println!(
                "slippage_basis_points is none, use default slippage basis points: {}",
                DEFAULT_SLIPPAGE
            );
        }
        let executor = TradeFactory::create_executor(params.dex_type.clone());
        let protocol_params = params.extension_params;

        let sell_params = InternalSellParams {
            rpc: Some(self.rpc.clone()),
            payer: self.payer.clone(),
            mint: params.mint,
            token_amount: Some(params.token_amount),
            slippage_basis_points: params.slippage_basis_points,
            lookup_table_key: params.lookup_table_key,
            recent_blockhash: params.recent_blockhash,
            wait_transaction_confirmed: params.wait_transaction_confirmed,
            protocol_params: protocol_params.clone(),
            with_tip: params.with_tip,
            open_seed_optimize: params.open_seed_optimize,
            swqos_settings: self.swqos_settings.clone(),
            middleware_manager: self.middleware_manager.clone(),
            create_wsol_ata: params.create_wsol_ata,
            close_wsol_ata: params.close_wsol_ata,
            custom_cu_limit: params.custom_cu_limit,
        };

        // Validate protocol params
        let is_valid_params = match params.dex_type {
            DexType::PumpFun => protocol_params.as_any().downcast_ref::<PumpFunParams>().is_some(),
            DexType::PumpSwap => {
                protocol_params.as_any().downcast_ref::<PumpSwapParams>().is_some()
            }
            DexType::Bonk => protocol_params.as_any().downcast_ref::<BonkParams>().is_some(),
            DexType::RaydiumCpmm => {
                protocol_params.as_any().downcast_ref::<RaydiumCpmmParams>().is_some()
            }
            DexType::RaydiumAmmV4 => {
                protocol_params.as_any().downcast_ref::<RaydiumAmmV4Params>().is_some()
            }
        };

        if !is_valid_params {
            return Err(anyhow::anyhow!("Invalid protocol params for Trade"));
        }

        // Execute sell based on tip preference
        executor.sell_with_tip(sell_params).await
    }

    /// Execute a sell order for a percentage of the specified token amount
    ///
    /// This is a convenience function that calculates the exact amount to sell based on
    /// a percentage of the total token amount and then calls the `sell` function.
    ///
    /// # Arguments
    ///
    /// * `params` - Sell trade parameters (will be modified with calculated token amount)
    /// * `amount_token` - Total amount of tokens available (in smallest token units)
    /// * `percent` - Percentage of tokens to sell (1-100, where 100 = 100%)
    ///
    /// # Returns
    ///
    /// Returns `Ok(Signature)` with the transaction signature if the sell order is successfully executed,
    /// or an error if the transaction fails.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - `percent` is 0 or greater than 100
    /// - Invalid protocol parameters are provided for the specified DEX type
    /// - The transaction fails to execute
    /// - Network or RPC errors occur
    /// - Insufficient token balance for the calculated sale amount
    /// - Token account doesn't exist or is not properly initialized
    /// - Required accounts cannot be created or accessed
    pub async fn sell_by_percent(
        &self,
        mut params: TradeSellParams,
        amount_token: u64,
        percent: u64,
    ) -> Result<Signature, anyhow::Error> {
        if percent == 0 || percent > 100 {
            return Err(anyhow::anyhow!("Percentage must be between 1 and 100"));
        }
        let amount = amount_token * percent / 100;
        params.token_amount = amount;
        self.sell(params).await
    }

    /// Wraps native SOL into wSOL (Wrapped SOL) for use in SPL token operations
    ///
    /// This function creates a wSOL associated token account (if it doesn't exist),
    /// transfers the specified amount of SOL to that account, and then syncs the native
    /// token balance to make SOL usable as an SPL token in trading operations.
    ///
    /// # Arguments
    /// * `amount` - The amount of SOL to wrap (in lamports)
    ///
    /// # Returns
    /// * `Ok(String)` - Transaction signature if successful
    /// * `Err(anyhow::Error)` - If the transaction fails to execute
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - Insufficient SOL balance for the wrap operation
    /// - wSOL associated token account creation fails
    /// - Transaction fails to execute or confirm
    /// - Network or RPC errors occur
    pub async fn wrap_sol_to_wsol(&self, amount: u64) -> Result<String, anyhow::Error> {
        use crate::trading::common::wsol_manager::handle_wsol;
        use solana_sdk::transaction::Transaction;
        let recent_blockhash = self.rpc.get_latest_blockhash().await?;
        let instructions = handle_wsol(&self.payer.pubkey(), amount);
        let mut transaction =
            Transaction::new_with_payer(&instructions, Some(&self.payer.pubkey()));
        transaction.sign(&[&*self.payer], recent_blockhash);
        let signature = self.rpc.send_and_confirm_transaction(&transaction).await?;
        Ok(signature.to_string())
    }
    /// Closes the wSOL associated token account and unwraps remaining balance to native SOL
    ///
    /// This function closes the wSOL associated token account, which automatically
    /// transfers any remaining wSOL balance back to the account owner as native SOL.
    /// This is useful for cleaning up wSOL accounts and recovering wrapped SOL after trading operations.
    ///
    /// # Returns
    /// * `Ok(String)` - Transaction signature if successful
    /// * `Err(anyhow::Error)` - If the transaction fails to execute
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - wSOL associated token account doesn't exist
    /// - Account closure fails due to insufficient permissions
    /// - Transaction fails to execute or confirm
    /// - Network or RPC errors occur
    pub async fn close_wsol(&self) -> Result<String, anyhow::Error> {
        use crate::trading::common::wsol_manager::close_wsol;
        use solana_sdk::transaction::Transaction;
        let recent_blockhash = self.rpc.get_latest_blockhash().await?;
        let instructions = close_wsol(&self.payer.pubkey());
        let mut transaction =
            Transaction::new_with_payer(&instructions, Some(&self.payer.pubkey()));
        transaction.sign(&[&*self.payer], recent_blockhash);
        let signature = self.rpc.send_and_confirm_transaction(&transaction).await?;
        Ok(signature.to_string())
    }
}
