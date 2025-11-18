pub mod common;
pub mod constants;
pub mod instruction;
pub mod perf;
pub mod swqos;
pub mod trading;
pub mod utils;
use crate::common::nonce_cache::DurableNonceInfo;
use crate::common::GasFeeStrategy;
use crate::common::TradeConfig;
use crate::constants::trade::trade::DEFAULT_SLIPPAGE;
use crate::constants::SOL_TOKEN_ACCOUNT;
use crate::constants::USD1_TOKEN_ACCOUNT;
use crate::constants::WSOL_TOKEN_ACCOUNT;
use crate::constants::USDC_TOKEN_ACCOUNT;
use crate::swqos::SwqosClient;
use crate::swqos::SwqosConfig;
use crate::swqos::TradeType;
use crate::trading::core::params::{BonkParams, MeteoraDlmmParams, OrcaParams};
use crate::trading::core::params::MeteoraDammV2Params;
use crate::trading::core::params::PumpFunParams;
use crate::trading::core::params::PumpSwapParams;
use crate::trading::core::params::RaydiumAmmV4Params;
use crate::trading::core::params::RaydiumCpmmParams;
use crate::trading::core::traits::ProtocolParams;
use crate::trading::factory::DexType;
use crate::trading::MiddlewareManager;
use crate::trading::SwapParams;
use crate::trading::TradeFactory;
use common::SolanaRpcClient;
use parking_lot::Mutex;
use rustls::crypto::{ring::default_provider, CryptoProvider};
use solana_sdk::hash::Hash;
use solana_sdk::message::AddressLookupTableAccount;
use solana_sdk::signer::Signer;
use solana_sdk::{pubkey::Pubkey, signature::Keypair, signature::Signature};
use std::sync::Arc;

/// Type of the token to buy
#[derive(Clone, PartialEq)]
pub enum TradeTokenType {
    SOL,
    WSOL,
    USD1,
    USDC,
}

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
    /// SWQOS clients for transaction priority and routing
    pub swqos_clients: Vec<Arc<SwqosClient>>,
    /// Optional middleware manager for custom transaction processing
    pub middleware_manager: Option<Arc<MiddlewareManager>>,
    /// Whether to use seed optimization for all ATA operations (default: true)
    /// Applies to all token account creations across buy and sell operations
    pub use_seed_optimize: bool,
}

static INSTANCE: Mutex<Option<Arc<SolanaTrade>>> = Mutex::new(None);

impl Clone for SolanaTrade {
    fn clone(&self) -> Self {
        Self {
            payer: self.payer.clone(),
            rpc: self.rpc.clone(),
            swqos_clients: self.swqos_clients.clone(),
            middleware_manager: self.middleware_manager.clone(),
            use_seed_optimize: self.use_seed_optimize,
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
    /// Type of the token to buy
    pub input_token_type: TradeTokenType,
    /// Public key of the token to purchase
    pub mint: Pubkey,
    /// Amount of tokens to buy (in smallest token units)
    pub input_token_amount: u64,
    /// Optional slippage tolerance in basis points (e.g., 100 = 1%)
    pub slippage_basis_points: Option<u64>,
    /// Recent blockhash for transaction validity
    pub recent_blockhash: Option<Hash>,
    /// Protocol-specific parameters (PumpFun, Raydium, etc.)
    pub extension_params: Box<dyn ProtocolParams>,
    // Extended configuration
    /// Optional address lookup table for transaction size optimization
    pub address_lookup_table_account: Option<AddressLookupTableAccount>,
    /// Whether to wait for transaction confirmation before returning
    pub wait_transaction_confirmed: bool,
    /// Whether to create input token associated token account
    pub create_input_token_ata: bool,
    /// Whether to close input token associated token account after trade
    pub close_input_token_ata: bool,
    /// Whether to create token mint associated token account
    pub create_mint_ata: bool,
    /// Durable nonce information
    pub durable_nonce: Option<DurableNonceInfo>,
    /// Optional fixed output token amount (If this value is set, it will be directly assigned to the output amount instead of being calculated)
    pub fixed_output_token_amount: Option<u64>,
    /// Gas fee strategy
    pub gas_fee_strategy: GasFeeStrategy,
    /// Whether to simulate the transaction instead of executing it
    pub simulate: bool,
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
    /// Type of the token to sell
    pub output_token_type: TradeTokenType,
    /// Public key of the token to sell
    pub mint: Pubkey,
    /// Amount of tokens to sell (in smallest token units)
    pub input_token_amount: u64,
    /// Optional slippage tolerance in basis points (e.g., 100 = 1%)
    pub slippage_basis_points: Option<u64>,
    /// Recent blockhash for transaction validity
    pub recent_blockhash: Option<Hash>,
    /// Whether to include tip for transaction priority
    pub with_tip: bool,
    /// Protocol-specific parameters (PumpFun, Raydium, etc.)
    pub extension_params: Box<dyn ProtocolParams>,
    // Extended configuration
    /// Optional address lookup table for transaction size optimization
    pub address_lookup_table_account: Option<AddressLookupTableAccount>,
    /// Whether to wait for transaction confirmation before returning
    pub wait_transaction_confirmed: bool,
    /// Whether to create output token associated token account
    pub create_output_token_ata: bool,
    /// Whether to close output token associated token account after trade
    pub close_output_token_ata: bool,
    /// Whether to close mint token associated token account after trade
    pub close_mint_token_ata: bool,
    /// Durable nonce information
    pub durable_nonce: Option<DurableNonceInfo>,
    /// Optional fixed output token amount (If this value is set, it will be directly assigned to the output amount instead of being calculated)
    pub fixed_output_token_amount: Option<u64>,
    /// Gas fee strategy
    pub gas_fee_strategy: GasFeeStrategy,
    /// Whether to simulate the transaction instead of executing it
    pub simulate: bool,
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
    pub async fn new(payer: Arc<Keypair>, trade_config: TradeConfig) -> Self {
        crate::common::fast_fn::fast_init(&payer.try_pubkey().unwrap());

        if CryptoProvider::get_default().is_none() {
            let _ = default_provider()
                .install_default()
                .map_err(|e| anyhow::anyhow!("Failed to install crypto provider: {:?}", e));
        }

        let rpc_url = trade_config.rpc_url.clone();
        let swqos_configs = trade_config.swqos_configs.clone();
        let commitment = trade_config.commitment.clone();
        let mut swqos_clients: Vec<Arc<SwqosClient>> = vec![];

        for swqos in swqos_configs {
            let swqos_client =
                SwqosConfig::get_swqos_client(rpc_url.clone(), commitment.clone(), swqos.clone());
            swqos_clients.push(swqos_client);
        }

        let rpc =
            Arc::new(SolanaRpcClient::new_with_commitment(rpc_url.clone(), commitment.clone()));
        common::seed::update_rents(&rpc).await.unwrap();
        common::seed::start_rent_updater(rpc.clone());

        // 🔧 初始化WSOL ATA：如果配置为启动时创建，则检查并创建
        if trade_config.create_wsol_ata_on_startup {
            // 根据seed配置计算WSOL ATA地址
            let wsol_ata =
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &payer.pubkey(),
                    &WSOL_TOKEN_ACCOUNT,
                    &crate::constants::TOKEN_PROGRAM,
                );

            // 查询账户是否存在
            match rpc.get_account(&wsol_ata).await {
                Ok(_) => {
                    // WSOL ATA已存在
                    println!("✅ WSOL ATA已存在: {}", wsol_ata);
                }
                Err(_) => {
                    // WSOL ATA不存在，创建它
                    println!("🔨 创建WSOL ATA: {}", wsol_ata);
                    // 使用seed优化创建WSOL ATA
                    let create_ata_ixs =
                        crate::trading::common::wsol_manager::create_wsol_ata(&payer.pubkey());

                    if !create_ata_ixs.is_empty() {
                        // 构建并发送交易
                        use solana_sdk::transaction::Transaction;
                        let recent_blockhash = rpc.get_latest_blockhash().await.unwrap();
                        let tx = Transaction::new_signed_with_payer(
                            &create_ata_ixs,
                            Some(&payer.pubkey()),
                            &[payer.as_ref()],
                            recent_blockhash,
                        );

                        match rpc.send_and_confirm_transaction(&tx).await {
                            Ok(signature) => {
                                println!("✅ WSOL ATA创建成功: {}", signature);
                            }
                            Err(e) => {
                                // 创建失败，检查是否是因为已存在
                                match rpc.get_account(&wsol_ata).await {
                                    Ok(_) => {
                                        println!(
                                            "✅ WSOL ATA已存在（交易失败但账户存在）: {}",
                                            wsol_ata
                                        );
                                    }
                                    Err(_) => {
                                        // 账户不存在且创建失败 - 这是严重错误，应该让启动失败
                                        panic!(
                                            "❌ WSOL ATA创建失败且账户不存在: {}. 错误: {}",
                                            wsol_ata, e
                                        );
                                    }
                                }
                            }
                        }
                    } else {
                        println!("ℹ️ WSOL ATA已存在（无需创建）");
                    }
                }
            }
        }

        let instance = Self {
            payer,
            rpc,
            swqos_clients,
            middleware_manager: None,
            use_seed_optimize: trade_config.use_seed_optimize,
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
    pub async fn buy(&self, params: TradeBuyParams) -> Result<(bool, Signature), anyhow::Error> {
        if params.slippage_basis_points.is_none() {
            println!(
                "slippage_basis_points is none, use default slippage basis points: {}",
                DEFAULT_SLIPPAGE
            );
        }
        if params.input_token_type == TradeTokenType::USD1 && params.dex_type != DexType::Bonk {
            return Err(anyhow::anyhow!(
                " Current version only support USD1 trading on Bonk protocols"
            ));
        }
        let input_token_mint = if params.input_token_type == TradeTokenType::SOL {
            SOL_TOKEN_ACCOUNT
        } else if params.input_token_type == TradeTokenType::WSOL {
            WSOL_TOKEN_ACCOUNT
        } else if params.input_token_type == TradeTokenType::USDC {
            USDC_TOKEN_ACCOUNT
        } else {
            USD1_TOKEN_ACCOUNT
        };
        let executor = TradeFactory::create_executor(params.dex_type.clone());
        let protocol_params = params.extension_params;
        let buy_params = SwapParams {
            rpc: Some(self.rpc.clone()),
            payer: self.payer.clone(),
            trade_type: TradeType::Buy,
            input_mint: input_token_mint,
            output_mint: params.mint,
            input_token_program: None,
            output_token_program: None,
            input_amount: Some(params.input_token_amount),
            slippage_basis_points: params.slippage_basis_points,
            address_lookup_table_account: params.address_lookup_table_account,
            recent_blockhash: params.recent_blockhash,
            data_size_limit: params.gas_fee_strategy.get_strategies(TradeType::Buy)
                .get(0)
                .map(|(_, _, v)| v.data_size_limit)
                .unwrap_or(256 * 1024),
            wait_transaction_confirmed: params.wait_transaction_confirmed,
            protocol_params: protocol_params.clone(),
            open_seed_optimize: self.use_seed_optimize, // 使用全局seed优化配置
            swqos_clients: self.swqos_clients.clone(),
            middleware_manager: self.middleware_manager.clone(),
            durable_nonce: params.durable_nonce,
            with_tip: true,
            create_input_mint_ata: params.create_input_token_ata,
            close_input_mint_ata: params.close_input_token_ata,
            create_output_mint_ata: params.create_mint_ata,
            close_output_mint_ata: false,
            fixed_output_amount: params.fixed_output_token_amount,
            gas_fee_strategy: params.gas_fee_strategy,
            simulate: params.simulate,
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
            DexType::MeteoraDammV2 => {
                protocol_params.as_any().downcast_ref::<MeteoraDammV2Params>().is_some()
            }
            DexType::MeteoraDlmm => {
                protocol_params.as_any().downcast_ref::<MeteoraDlmmParams>().is_some()
            }
            DexType::Orca => {
                protocol_params.as_any().downcast_ref::<OrcaParams>().is_some()
            }
        };

        if !is_valid_params {
            return Err(anyhow::anyhow!("Invalid protocol params for Trade"));
        }

        executor.swap(buy_params).await
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
    pub async fn sell(&self, params: TradeSellParams) -> Result<(bool, Signature), anyhow::Error> {
        if params.slippage_basis_points.is_none() {
            println!(
                "slippage_basis_points is none, use default slippage basis points: {}",
                DEFAULT_SLIPPAGE
            );
        }
        if params.output_token_type == TradeTokenType::USD1 && params.dex_type != DexType::Bonk {
            return Err(anyhow::anyhow!(
                " Current version only support USD1 trading on Bonk protocols"
            ));
        }
        let executor = TradeFactory::create_executor(params.dex_type.clone());
        let protocol_params = params.extension_params;
        let output_token_mint = if params.output_token_type == TradeTokenType::SOL {
            SOL_TOKEN_ACCOUNT
        } else if params.output_token_type == TradeTokenType::WSOL {
            WSOL_TOKEN_ACCOUNT
        } else if params.output_token_type == TradeTokenType::USDC {
            USDC_TOKEN_ACCOUNT
        } else {
            USD1_TOKEN_ACCOUNT
        };
        let sell_params = SwapParams {
            rpc: Some(self.rpc.clone()),
            payer: self.payer.clone(),
            trade_type: TradeType::Sell,
            input_mint: params.mint,
            output_mint: output_token_mint,
            input_token_program: None,
            output_token_program: None,
            input_amount: Some(params.input_token_amount),
            slippage_basis_points: params.slippage_basis_points,
            address_lookup_table_account: params.address_lookup_table_account,
            recent_blockhash: params.recent_blockhash,
            wait_transaction_confirmed: params.wait_transaction_confirmed,
            protocol_params: protocol_params.clone(),
            with_tip: params.with_tip,
            open_seed_optimize: self.use_seed_optimize, // 使用全局seed优化配置
            swqos_clients: self.swqos_clients.clone(),
            middleware_manager: self.middleware_manager.clone(),
            durable_nonce: params.durable_nonce,
            data_size_limit: params.gas_fee_strategy.get_strategies(TradeType::Sell)
                .get(0)
                .map(|(_, _, v)| v.data_size_limit)
                .unwrap_or(0),
            create_input_mint_ata: false,
            close_input_mint_ata: params.close_mint_token_ata,
            create_output_mint_ata: params.create_output_token_ata,
            close_output_mint_ata: params.close_output_token_ata,
            fixed_output_amount: params.fixed_output_token_amount,
            gas_fee_strategy: params.gas_fee_strategy,
            simulate: params.simulate,
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
            DexType::MeteoraDammV2 => {
                protocol_params.as_any().downcast_ref::<MeteoraDammV2Params>().is_some()
            }
            DexType::MeteoraDlmm => {
                protocol_params.as_any().downcast_ref::<MeteoraDlmmParams>().is_some()
            }
            DexType::Orca => {
                protocol_params.as_any().downcast_ref::<OrcaParams>().is_some()
            }
        };

        if !is_valid_params {
            return Err(anyhow::anyhow!("Invalid protocol params for Trade"));
        }

        // Execute sell based on tip preference
        executor.swap(sell_params).await
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
    ) -> Result<(bool, Signature), anyhow::Error> {
        if percent == 0 || percent > 100 {
            return Err(anyhow::anyhow!("Percentage must be between 1 and 100"));
        }
        let amount = amount_token * percent / 100;
        params.input_token_amount = amount;
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

    /// Creates a wSOL associated token account (ATA) without wrapping any SOL
    ///
    /// This function only creates the wSOL associated token account for the payer
    /// without transferring any SOL into it. This is useful when you want to set up
    /// the account infrastructure in advance without committing funds yet.
    ///
    /// # Returns
    /// * `Ok(String)` - Transaction signature if successful
    /// * `Err(anyhow::Error)` - If the transaction fails to execute
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - wSOL ATA account already exists (idempotent, will succeed silently)
    /// - Transaction fails to execute or confirm
    /// - Network or RPC errors occur
    /// - Insufficient SOL for transaction fees
    pub async fn create_wsol_ata(&self) -> Result<String, anyhow::Error> {
        use crate::trading::common::wsol_manager::create_wsol_ata;
        use solana_sdk::transaction::Transaction;

        let recent_blockhash = self.rpc.get_latest_blockhash().await?;
        let instructions = create_wsol_ata(&self.payer.pubkey());

        // If instructions are empty, ATA already exists
        if instructions.is_empty() {
            return Err(anyhow::anyhow!(
                "wSOL ATA already exists or no instructions needed"
            ));
        }

        let mut transaction =
            Transaction::new_with_payer(&instructions, Some(&self.payer.pubkey()));
        transaction.sign(&[&*self.payer], recent_blockhash);
        let signature = self.rpc.send_and_confirm_transaction(&transaction).await?;
        Ok(signature.to_string())
    }
}
