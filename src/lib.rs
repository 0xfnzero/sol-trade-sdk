pub mod common;
pub mod constants;
pub mod instruction;
pub mod perf;
pub mod swqos;
pub mod trading;
pub mod utils;
use crate::common::nonce_cache::DurableNonceInfo;
use crate::common::GasFeeStrategy;
use crate::common::{TradeConfig, InfrastructureConfig};
use crate::constants::trade::trade::DEFAULT_SLIPPAGE;
use crate::constants::SOL_TOKEN_ACCOUNT;
use crate::constants::USD1_TOKEN_ACCOUNT;
use crate::constants::USDC_TOKEN_ACCOUNT;
use crate::constants::WSOL_TOKEN_ACCOUNT;
use crate::swqos::common::TradeError;
use crate::swqos::SwqosClient;
use crate::swqos::SwqosConfig;
use crate::swqos::TradeType;
use crate::trading::core::params::BonkParams;
use crate::trading::core::params::MeteoraDammV2Params;
use crate::trading::core::params::PumpFunParams;
use crate::trading::core::params::PumpSwapParams;
use crate::trading::core::params::RaydiumAmmV4Params;
use crate::trading::core::params::RaydiumCpmmParams;
use crate::trading::core::params::DexParamEnum;
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

/// Shared infrastructure components that can be reused across multiple wallets
///
/// This struct holds the expensive-to-initialize components (RPC client, SWQOS clients)
/// that are wallet-independent and can be shared when only the trading wallet changes.
pub struct TradingInfrastructure {
    /// Shared RPC client for blockchain interactions
    pub rpc: Arc<SolanaRpcClient>,
    /// Shared SWQOS clients for transaction priority and routing
    pub swqos_clients: Vec<Arc<SwqosClient>>,
    /// Configuration used to create this infrastructure
    pub config: InfrastructureConfig,
}

impl TradingInfrastructure {
    /// Create new shared infrastructure from configuration
    ///
    /// This performs the expensive initialization:
    /// - Creates RPC client with connection pool
    /// - Creates SWQOS clients (each with their own HTTP client)
    /// - Initializes rent cache and starts background updater
    pub async fn new(config: InfrastructureConfig) -> Self {
        // Install crypto provider (idempotent)
        if CryptoProvider::get_default().is_none() {
            let _ = default_provider()
                .install_default()
                .map_err(|e| anyhow::anyhow!("Failed to install crypto provider: {:?}", e));
        }

        // Create RPC client
        let rpc = Arc::new(SolanaRpcClient::new_with_commitment(
            config.rpc_url.clone(),
            config.commitment.clone(),
        ));

        // Initialize rent cache and start background updater
        common::seed::update_rents(&rpc).await.unwrap();
        common::seed::start_rent_updater(rpc.clone());

        // Create SWQOS clients with blacklist checking
        let mut swqos_clients: Vec<Arc<SwqosClient>> = vec![];
        for swqos in &config.swqos_configs {
            // Check blacklist, skip disabled providers
            if swqos.is_blacklisted() {
                eprintln!("\u{26a0}\u{fe0f} SWQOS {:?} is blacklisted, skipping", swqos.swqos_type());
                continue;
            }
            match SwqosConfig::get_swqos_client(
                config.rpc_url.clone(),
                config.commitment.clone(),
                swqos.clone(),
            ).await {
                Ok(swqos_client) => swqos_clients.push(swqos_client),
                Err(err) => eprintln!(
                    "failed to create {:?} swqos client: {err}. Excluding from swqos list",
                    swqos.swqos_type()
                ),
            }
        }

        Self {
            rpc,
            swqos_clients,
            config,
        }
    }
}

/// Main trading client for Solana DeFi protocols
///
/// `SolTradingSDK` provides a unified interface for trading across multiple Solana DEXs
/// including PumpFun, PumpSwap, Bonk, Raydium AMM V4, and Raydium CPMM.
/// It manages RPC connections, transaction signing, and SWQOS (Solana Web Quality of Service) settings.
pub struct TradingClient {
    /// The keypair used for signing all transactions
    pub payer: Arc<Keypair>,
    /// Shared infrastructure (RPC client, SWQOS clients)
    /// Can be shared across multiple TradingClient instances with different wallets
    pub infrastructure: Arc<TradingInfrastructure>,
    /// Optional middleware manager for custom transaction processing
    pub middleware_manager: Option<Arc<MiddlewareManager>>,
    /// Whether to use seed optimization for all ATA operations (default: true)
    /// Applies to all token account creations across buy and sell operations
    pub use_seed_optimize: bool,
}

static INSTANCE: Mutex<Option<Arc<TradingClient>>> = Mutex::new(None);

/// 🔄 向后兼容：SolanaTrade 别名
pub type SolanaTrade = TradingClient;

impl Clone for TradingClient {
    fn clone(&self) -> Self {
        Self {
            payer: self.payer.clone(),
            infrastructure: self.infrastructure.clone(),
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
    pub extension_params: DexParamEnum,
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
    pub extension_params: DexParamEnum,
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

impl TradingClient {
    /// Create a TradingClient from shared infrastructure (fast path)
    ///
    /// This is the preferred method when multiple wallets share the same infrastructure.
    /// It only performs wallet-specific initialization (fast_init) without the expensive
    /// RPC/SWQOS client creation.
    ///
    /// # Arguments
    /// * `payer` - The keypair used for signing transactions
    /// * `infrastructure` - Shared infrastructure (RPC client, SWQOS clients)
    /// * `use_seed_optimize` - Whether to use seed optimization for ATA operations
    ///
    /// # Returns
    /// Returns a configured `TradingClient` instance ready for trading operations
    pub fn from_infrastructure(
        payer: Arc<Keypair>,
        infrastructure: Arc<TradingInfrastructure>,
        use_seed_optimize: bool,
    ) -> Self {
        // Initialize wallet-specific caches (fast, synchronous)
        crate::common::fast_fn::fast_init(&payer.pubkey());

        Self {
            payer,
            infrastructure,
            middleware_manager: None,
            use_seed_optimize,
        }
    }

    /// Create a TradingClient from shared infrastructure with optional WSOL ATA setup
    ///
    /// Same as `from_infrastructure` but also handles WSOL ATA creation if requested.
    ///
    /// # Arguments
    /// * `payer` - The keypair used for signing transactions
    /// * `infrastructure` - Shared infrastructure (RPC client, SWQOS clients)
    /// * `use_seed_optimize` - Whether to use seed optimization for ATA operations
    /// * `create_wsol_ata` - Whether to check/create WSOL ATA
    pub async fn from_infrastructure_with_wsol_setup(
        payer: Arc<Keypair>,
        infrastructure: Arc<TradingInfrastructure>,
        use_seed_optimize: bool,
        create_wsol_ata: bool,
    ) -> Self {
        crate::common::fast_fn::fast_init(&payer.pubkey());

        if create_wsol_ata {
            Self::ensure_wsol_ata(&payer, &infrastructure.rpc).await;
        }

        Self {
            payer,
            infrastructure,
            middleware_manager: None,
            use_seed_optimize,
        }
    }

    /// Helper to ensure WSOL ATA exists for a wallet
    async fn ensure_wsol_ata(payer: &Arc<Keypair>, rpc: &Arc<SolanaRpcClient>) {
        let wsol_ata =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                &payer.pubkey(),
                &WSOL_TOKEN_ACCOUNT,
                &crate::constants::TOKEN_PROGRAM,
            );

        match rpc.get_account(&wsol_ata).await {
            Ok(_) => {
                println!("✅ WSOL ATA已存在: {}", wsol_ata);
            }
            Err(_) => {
                println!("🔨 创建WSOL ATA: {}", wsol_ata);
                let create_ata_ixs =
                    crate::trading::common::wsol_manager::create_wsol_ata(&payer.pubkey());

                if !create_ata_ixs.is_empty() {
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
                            match rpc.get_account(&wsol_ata).await {
                                Ok(_) => {
                                    println!(
                                        "✅ WSOL ATA已存在（交易失败但账户存在）: {}",
                                        wsol_ata
                                    );
                                }
                                Err(_) => {
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

    /// Creates a new SolTradingSDK instance with the specified configuration
    ///
    /// This function initializes the trading system with RPC connection, SWQOS settings,
    /// and sets up necessary components for trading operations.
    ///
    /// # Arguments
    /// * `payer` - The keypair used for signing transactions
    /// * `trade_config` - Trading configuration including RPC URL, SWQOS settings, etc.
    ///
    /// # Returns
    /// Returns a configured `SolTradingSDK` instance ready for trading operations
    #[inline]
    pub async fn new(payer: Arc<Keypair>, trade_config: TradeConfig) -> Self {
        // Create infrastructure from trade config
        let infra_config = InfrastructureConfig::from_trade_config(&trade_config);
        let infrastructure = Arc::new(TradingInfrastructure::new(infra_config).await);

        // Initialize wallet-specific caches
        crate::common::fast_fn::fast_init(&payer.pubkey());

        // Handle WSOL ATA creation if configured
        if trade_config.create_wsol_ata_on_startup {
            Self::ensure_wsol_ata(&payer, &infrastructure.rpc).await;
        }

        let instance = Self {
            payer,
            infrastructure,
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
        &self.infrastructure.rpc
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
    /// 🔧 修复：返回Vec<Signature>支持多SWQOS并发交易
    /// - bool: 是否至少有一个交易成功
    /// - Vec<Signature>: 所有提交的交易签名（按SWQOS顺序）
    /// - Option<TradeError>: 最后一个错误（如果全部失败）
    ///
    /// # Arguments
    ///
    /// * `params` - Buy trade parameters containing all necessary trading configuration
    ///
    /// # Returns
    ///
    /// Returns `Ok((bool, Vec<Signature>, Option<TradeError>))` with success flag and all transaction signatures,
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
    #[inline]
    pub async fn buy(
        &self,
        params: TradeBuyParams,
    ) -> Result<(bool, Vec<Signature>, Option<TradeError>), anyhow::Error> {
        #[cfg(feature = "perf-trace")]
        if params.slippage_basis_points.is_none() {
            log::debug!(
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
            rpc: Some(self.infrastructure.rpc.clone()),
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
            wait_transaction_confirmed: params.wait_transaction_confirmed,
            protocol_params: protocol_params.clone(),
            open_seed_optimize: self.use_seed_optimize, // 使用全局seed优化配置
            swqos_clients: self.infrastructure.swqos_clients.clone(),
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
        };

        if !is_valid_params {
            return Err(anyhow::anyhow!("Invalid protocol params for Trade"));
        }

        let swap_result = executor.swap(buy_params).await;
        let result =
            swap_result.map(|(success, sigs, err)| (success, sigs, err.map(TradeError::from)));
        return result;
    }

    /// Execute a sell order for a specified token
    ///
    /// 🔧 修复：返回Vec<Signature>支持多SWQOS并发交易
    /// - bool: 是否至少有一个交易成功
    /// - Vec<Signature>: 所有提交的交易签名（按SWQOS顺序）
    /// - Option<TradeError>: 最后一个错误（如果全部失败）
    ///
    /// # Arguments
    ///
    /// * `params` - Sell trade parameters containing all necessary trading configuration
    ///
    /// # Returns
    ///
    /// Returns `Ok((bool, Vec<Signature>, Option<TradeError>))` with success flag and all transaction signatures,
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
    #[inline]
    pub async fn sell(
        &self,
        params: TradeSellParams,
    ) -> Result<(bool, Vec<Signature>, Option<TradeError>), anyhow::Error> {
        #[cfg(feature = "perf-trace")]
        if params.slippage_basis_points.is_none() {
            log::debug!(
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
            rpc: Some(self.infrastructure.rpc.clone()),
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
            swqos_clients: self.infrastructure.swqos_clients.clone(),
            middleware_manager: self.middleware_manager.clone(),
            durable_nonce: params.durable_nonce,
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
        };

        if !is_valid_params {
            return Err(anyhow::anyhow!("Invalid protocol params for Trade"));
        }

        // Execute sell based on tip preference
        let swap_result = executor.swap(sell_params).await;
        let result =
            swap_result.map(|(success, sigs, err)| (success, sigs, err.map(TradeError::from)));
        return result;
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
    ) -> Result<(bool, Vec<Signature>, Option<TradeError>), anyhow::Error> {
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
        let recent_blockhash = self.infrastructure.rpc.get_latest_blockhash().await?;
        let instructions = handle_wsol(&self.payer.pubkey(), amount);
        let mut transaction =
            Transaction::new_with_payer(&instructions, Some(&self.payer.pubkey()));
        transaction.sign(&[&*self.payer], recent_blockhash);
        let signature = self.infrastructure.rpc.send_and_confirm_transaction(&transaction).await?;
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
        let recent_blockhash = self.infrastructure.rpc.get_latest_blockhash().await?;
        let instructions = close_wsol(&self.payer.pubkey());
        let mut transaction =
            Transaction::new_with_payer(&instructions, Some(&self.payer.pubkey()));
        transaction.sign(&[&*self.payer], recent_blockhash);
        let signature = self.infrastructure.rpc.send_and_confirm_transaction(&transaction).await?;
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

        let recent_blockhash = self.infrastructure.rpc.get_latest_blockhash().await?;
        let instructions = create_wsol_ata(&self.payer.pubkey());

        // If instructions are empty, ATA already exists
        if instructions.is_empty() {
            return Err(anyhow::anyhow!("wSOL ATA already exists or no instructions needed"));
        }

        let mut transaction =
            Transaction::new_with_payer(&instructions, Some(&self.payer.pubkey()));
        transaction.sign(&[&*self.payer], recent_blockhash);
        let signature = self.infrastructure.rpc.send_and_confirm_transaction(&transaction).await?;
        Ok(signature.to_string())
    }

    /// 将 WSOL 转换为 SOL，使用 seed 账户
    ///
    /// 这个函数实现以下步骤：
    /// 1. 使用 super::seed::create_associated_token_account_use_seed 创建 WSOL seed 账号
    /// 2. 使用 get_associated_token_address_with_program_id_use_seed 获取该账号的 ATA 地址
    /// 3. 添加从用户 WSOL ATA 转账到该 seed ATA 账号的指令
    /// 4. 添加关闭 WSOL seed 账号的指令
    ///
    /// # Arguments
    /// * `amount` - 要转换的 WSOL 数量（以 lamports 为单位）
    ///
    /// # Returns
    /// * `Ok(String)` - 交易签名
    /// * `Err(anyhow::Error)` - 如果交易执行失败
    ///
    /// # Errors
    ///
    /// 此函数在以下情况下会返回错误：
    /// - 用户 WSOL ATA 中余额不足
    /// - seed 账户创建失败
    /// - 转账指令执行失败
    /// - 交易执行或确认失败
    /// - 网络或 RPC 错误
    pub async fn wrap_wsol_to_sol(&self, amount: u64) -> Result<String, anyhow::Error> {
        use crate::trading::common::wsol_manager::{wrap_wsol_to_sol as wrap_wsol_to_sol_internal, wrap_wsol_to_sol_without_create};
        use crate::common::seed::get_associated_token_address_with_program_id_use_seed;
        use solana_sdk::transaction::Transaction;

        // 检查临时seed账户是否已存在
        let seed_ata_address = get_associated_token_address_with_program_id_use_seed(
            &self.payer.pubkey(),
            &crate::constants::WSOL_TOKEN_ACCOUNT,
            &crate::constants::TOKEN_PROGRAM,
        )?;

        let account_exists = self.infrastructure.rpc.get_account(&seed_ata_address).await.is_ok();

        let instructions = if account_exists {
            // 如果账户已存在，使用不创建账户的版本
            wrap_wsol_to_sol_without_create(&self.payer.pubkey(), amount)?
        } else {
            // 如果账户不存在，使用创建账户的版本
            wrap_wsol_to_sol_internal(&self.payer.pubkey(), amount)?
        };

        let recent_blockhash = self.infrastructure.rpc.get_latest_blockhash().await?;
        let mut transaction = Transaction::new_with_payer(&instructions, Some(&self.payer.pubkey()));
        transaction.sign(&[&*self.payer], recent_blockhash);
        let signature = self.infrastructure.rpc.send_and_confirm_transaction(&transaction).await?;
        Ok(signature.to_string())
    }
}
