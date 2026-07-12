//! High-level [`TradingClient`], [`TradingInfrastructure`], and trade parameter types.

use crate::common::nonce_cache::DurableNonceInfo;
use crate::common::sdk_log;
use crate::common::GasFeeStrategy;
use crate::common::SolanaRpcClient;
use crate::common::{InfrastructureConfig, TradeConfig};
#[cfg(feature = "perf-trace")]
use crate::constants::trade::trade::DEFAULT_SLIPPAGE;
use crate::constants::SOL_TOKEN_ACCOUNT;
use crate::constants::USD1_TOKEN_ACCOUNT;
use crate::constants::USDC_TOKEN_ACCOUNT;
use crate::constants::WSOL_TOKEN_ACCOUNT;
use crate::swqos::common::TradeError;
use crate::swqos::SwqosClient;
use crate::swqos::SwqosConfig;
use crate::swqos::SwqosType;
use crate::swqos::TradeType;
use crate::trading::core::params::BonkParams;
use crate::trading::core::params::DexParamEnum;
use crate::trading::core::params::MeteoraDammV2Params;
use crate::trading::core::params::PumpFunParams;
use crate::trading::core::params::PumpSwapParams;
use crate::trading::core::params::RaydiumAmmV4Params;
use crate::trading::core::params::RaydiumCpmmParams;
use crate::trading::factory::DexType;
use crate::trading::MiddlewareManager;
use crate::trading::SwapParams;
use crate::trading::TradeFactory;
use parking_lot::Mutex;
use rustls::crypto::{ring::default_provider, CryptoProvider};
use solana_sdk::hash::Hash;
use solana_sdk::message::AddressLookupTableAccount;
use solana_sdk::signer::Signer;
use solana_sdk::{pubkey::Pubkey, signature::Keypair, signature::Signature};
use std::sync::Arc;
#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

/// Single place to validate that protocol params match the given DEX type (avoids duplicate match in buy/sell).
#[inline(always)]
fn validate_protocol_params(dex_type: DexType, params: &DexParamEnum) -> bool {
    match dex_type {
        DexType::PumpFun => params.as_any().downcast_ref::<PumpFunParams>().is_some(),
        DexType::PumpSwap => params.as_any().downcast_ref::<PumpSwapParams>().is_some(),
        DexType::Bonk => params.as_any().downcast_ref::<BonkParams>().is_some(),
        DexType::RaydiumCpmm => params.as_any().downcast_ref::<RaydiumCpmmParams>().is_some(),
        DexType::RaydiumAmmV4 => params.as_any().downcast_ref::<RaydiumAmmV4Params>().is_some(),
        DexType::MeteoraDammV2 => params.as_any().downcast_ref::<MeteoraDammV2Params>().is_some(),
    }
}

#[inline]
fn normalize_swqos_configs(rpc_url: &str, configs: &[SwqosConfig]) -> Vec<SwqosConfig> {
    let mut out = configs.to_vec();
    if !out.iter().any(|c| matches!(c.swqos_type(), SwqosType::Default)) {
        out.push(SwqosConfig::Default(rpc_url.to_string()));
    }
    out
}

/// 按 mint 查找池地址（通用入口，根据 DEX 类型分发，仅 PumpSwap 等已实现的类型会走优化路径）。
///
/// * `dex_type`：PumpSwap 时先走 PDA 再回退 getProgramAccounts，其他类型返回未实现错误。
pub async fn find_pool_by_mint(
    rpc: &SolanaRpcClient,
    mint: &Pubkey,
    dex_type: DexType,
) -> Result<Pubkey, anyhow::Error> {
    match dex_type {
        DexType::PumpSwap => crate::instruction::utils::pumpswap::find_pool(rpc, mint).await,
        _ => Err(anyhow::anyhow!("find_pool_by_mint not implemented for {:?}", dex_type)),
    }
}

/// Type of the token to buy
#[derive(Clone, PartialEq)]
pub enum TradeTokenType {
    SOL,
    WSOL,
    USD1,
    USDC,
}

/// Account lifecycle policy for high-level trade requests.
///
/// This replaces low-level flags such as `create_input_token_ata`,
/// `close_input_token_ata`, `create_mint_ata`, and `create_output_token_ata`.
/// Use this when calling [`TradingClient::buy_simple`] or [`TradingClient::sell_simple`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountPolicy {
    /// SDK chooses a practical default for the protocol and token route.
    ///
    /// Buy: create the target token ATA because the wallet usually needs a place
    /// to receive the bought token.
    ///
    /// Sell: create the output ATA only when receiving an SPL token such as USDC
    /// or WSOL. Receiving native SOL does not need an ATA.
    Auto,
    /// Keep the transaction small for sniping/arbitrage; assume hot accounts are prepared.
    ///
    /// This avoids ATA create/close instructions in the trade transaction. It is
    /// the recommended policy for latency-sensitive bots that pre-create WSOL
    /// and token ATAs outside the hot path.
    HotPathMinimal,
    /// Include ATA create instructions when the route needs them.
    ///
    /// This is more convenient for normal trading but can make PumpFun V2 and
    /// other large transactions exceed Solana packet size limits unless an ALT
    /// is supplied or the transaction builder can compact optional instructions.
    CreateMissing,
    /// Do not create or close token accounts in the trade transaction.
    ///
    /// Use this when the caller has already prepared all required ATAs and wants
    /// deterministic instruction layout. Unlike `HotPathMinimal`, this name is
    /// intended for correctness/readability rather than bot latency.
    AssumePrepared,
}

/// High-level buy sizing intent.
///
/// This replaces `input_token_amount`, `fixed_output_token_amount`, and
/// `use_exact_sol_amount` in [`TradeBuyParams`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuyAmount {
    /// Spend this exact quote amount and apply slippage to minimum output.
    ///
    /// Example: spend exactly `0.1 SOL` or exactly `100 USDC` worth of quote
    /// token; if the received token amount is below the slippage-adjusted
    /// minimum, the transaction fails.
    ExactInput(u64),
    /// Buy an exact token amount, using `max_input_amount` as the quote budget.
    ///
    /// Example: buy exactly `1_000_000` token base units, but fail if the quote
    /// cost would exceed `max_input_amount`.
    ExactOutput { output_amount: u64, max_input_amount: u64 },
    /// Regular PumpFun/PumpSwap buy: estimate output from `quote_amount` and apply slippage to max quote cost.
    ///
    /// This maps to `use_exact_sol_amount = Some(false)` in the old API. It is
    /// useful for bots that want to know the intended token output from the SDK
    /// calculation while letting the chain fail if the max quote budget is
    /// exceeded.
    WithMaxInput { quote_amount: u64 },
}

/// High-level sell sizing intent.
///
/// This replaces `input_token_amount` and `fixed_output_token_amount` in
/// [`TradeSellParams`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SellAmount {
    /// Sell this exact token amount and apply slippage to minimum output.
    ///
    /// Example: sell exactly `1_000_000` token base units and fail if the
    /// received quote amount is below the slippage-adjusted minimum.
    ExactInput(u64),
    /// Exact-output sell where supported; `max_input_amount` is the token budget.
    ///
    /// Example: receive exactly `0.1 SOL` or `100 USDC`, but spend no more than
    /// `max_input_amount` token base units.
    ExactOutput { output_amount: u64, max_input_amount: u64 },
}

/// Simpler buy request that describes trade intent instead of low-level ATA flags.
///
/// Prefer constructing this with [`SimpleBuyParams::new`] or
/// [`SimpleBuyParams::with_durable_nonce`], then optionally set slippage,
/// account policy, ALT, simulation, or confirmation flags through builder-style
/// methods.
#[derive(Clone)]
pub struct SimpleBuyParams {
    /// DEX/protocol to route through, such as `DexType::PumpFun`.
    pub dex_type: DexType,
    /// Quote token used to pay for the buy: `SOL`, `WSOL`, `USDC`, or `USD1`.
    ///
    /// For PumpFun SOL-paired coins, use `TradeTokenType::SOL` for the normal
    /// fast path even when parser data reports WSOL as the quote sentinel. The
    /// SDK will prefer V1; choose `WSOL` only when intentionally spending an
    /// existing WSOL ATA through V2.
    pub pay_with: TradeTokenType,
    /// Mint address of the token being bought.
    pub mint: Pubkey,
    /// Buy sizing intent. See [`BuyAmount`].
    pub amount: BuyAmount,
    /// Optional slippage in basis points. `100` means 1%.
    pub slippage_basis_points: Option<u64>,
    /// Recent blockhash for non-nonce transactions.
    ///
    /// The SDK intentionally does not fetch blockhash on the hot path. Use
    /// [`SimpleBuyParams::with_durable_nonce`] only when the caller specifically
    /// wants durable-nonce transactions.
    pub recent_blockhash: Option<Hash>,
    /// Protocol-specific parameters, for example `DexParamEnum::PumpFun(...)`.
    pub extension_params: DexParamEnum,
    /// Compute unit price/limit and relay tip configuration.
    pub gas_fee_strategy: GasFeeStrategy,
    /// ATA creation/close behavior. See [`AccountPolicy`].
    pub account_policy: AccountPolicy,
    /// Optional Address Lookup Tables to reduce transaction size.
    /// Pass one element for a single ALT or multiple elements for multi-ALT.
    pub address_lookup_table_accounts: Vec<AddressLookupTableAccount>,
    /// Wait until the transaction is confirmed before returning.
    pub wait_tx_confirmed: bool,
    /// Wait for every SWQoS route's submit response so all signatures can be
    /// returned. Useful when confirming through poll-any semantics or monitoring
    /// route variants externally. Recent-blockhash variants are not mutually
    /// exclusive; durable nonce variants are.
    pub wait_for_all_submits: bool,
    /// Durable nonce info. Mutually exclusive with `recent_blockhash`.
    pub durable_nonce: Option<DurableNonceInfo>,
    /// Build and simulate the transaction instead of submitting it.
    pub simulate: bool,
    /// Optional upstream receive timestamp in microseconds for latency tracing.
    pub grpc_recv_us: Option<i64>,
}

/// Simpler sell request that describes trade intent instead of low-level ATA flags.
///
/// Prefer constructing this with [`SimpleSellParams::new`] or
/// [`SimpleSellParams::with_durable_nonce`].
#[derive(Clone)]
pub struct SimpleSellParams {
    /// DEX/protocol to route through, such as `DexType::PumpFun`.
    pub dex_type: DexType,
    /// Quote token to receive from the sell: `SOL`, `WSOL`, `USDC`, or `USD1`.
    pub receive_as: TradeTokenType,
    /// Mint address of the token being sold.
    pub mint: Pubkey,
    /// Sell sizing intent. See [`SellAmount`].
    pub amount: SellAmount,
    /// Optional slippage in basis points. `100` means 1%.
    pub slippage_basis_points: Option<u64>,
    /// Recent blockhash for non-nonce transactions.
    pub recent_blockhash: Option<Hash>,
    /// Protocol-specific parameters, for example `DexParamEnum::PumpFun(...)`.
    pub extension_params: DexParamEnum,
    /// Compute unit price/limit and relay tip configuration.
    pub gas_fee_strategy: GasFeeStrategy,
    /// ATA creation/close behavior. See [`AccountPolicy`].
    pub account_policy: AccountPolicy,
    /// Optional Address Lookup Tables to reduce transaction size.
    /// Pass one element for a single ALT or multiple elements for multi-ALT.
    pub address_lookup_table_accounts: Vec<AddressLookupTableAccount>,
    /// Wait until the transaction is confirmed before returning.
    pub wait_tx_confirmed: bool,
    /// Wait for every SWQoS route's submit response so all signatures can be
    /// returned. Useful when confirming through poll-any semantics or monitoring
    /// route variants externally. Recent-blockhash variants are not mutually
    /// exclusive; durable nonce variants are.
    pub wait_for_all_submits: bool,
    /// Durable nonce info. Mutually exclusive with `recent_blockhash`.
    pub durable_nonce: Option<DurableNonceInfo>,
    /// Build and simulate the transaction instead of submitting it.
    pub simulate: bool,
    /// Whether to include relay tips for sell transactions.
    pub with_tip: bool,
    /// Optional upstream receive timestamp in microseconds for latency tracing.
    pub grpc_recv_us: Option<i64>,
}

impl SimpleBuyParams {
    /// Create a simple buy request using a recent blockhash.
    ///
    /// Defaults:
    /// - `account_policy = AccountPolicy::Auto`
    /// - `wait_tx_confirmed = false`
    /// - `wait_for_all_submits = false`
    /// - `simulate = false`
    /// - no ALT
    /// - no slippage override
    pub fn new(
        dex_type: DexType,
        pay_with: TradeTokenType,
        mint: Pubkey,
        amount: BuyAmount,
        extension_params: DexParamEnum,
        recent_blockhash: Hash,
        gas_fee_strategy: GasFeeStrategy,
    ) -> Self {
        Self {
            dex_type,
            pay_with,
            mint,
            amount,
            slippage_basis_points: None,
            recent_blockhash: Some(recent_blockhash),
            extension_params,
            gas_fee_strategy,
            account_policy: AccountPolicy::Auto,
            address_lookup_table_accounts: Vec::new(),
            wait_tx_confirmed: false,
            wait_for_all_submits: false,
            durable_nonce: None,
            simulate: false,
            grpc_recv_us: None,
        }
    }

    /// Create a simple buy request using a durable nonce instead of a recent blockhash.
    ///
    /// Use this when sending through multiple SWQoS lanes with the same nonce.
    pub fn with_durable_nonce(
        dex_type: DexType,
        pay_with: TradeTokenType,
        mint: Pubkey,
        amount: BuyAmount,
        extension_params: DexParamEnum,
        durable_nonce: DurableNonceInfo,
        gas_fee_strategy: GasFeeStrategy,
    ) -> Self {
        Self {
            durable_nonce: Some(durable_nonce),
            recent_blockhash: None,
            ..Self::new(
                dex_type,
                pay_with,
                mint,
                amount,
                extension_params,
                Hash::default(),
                gas_fee_strategy,
            )
        }
    }

    /// Set slippage in basis points. `100` means 1%.
    pub fn slippage_basis_points(mut self, value: u64) -> Self {
        self.slippage_basis_points = Some(value);
        self
    }

    /// Set account lifecycle behavior. Bots usually want `HotPathMinimal`;
    /// normal integrations can keep the default `Auto`.
    pub fn account_policy(mut self, value: AccountPolicy) -> Self {
        self.account_policy = value;
        self
    }

    /// Attach Address Lookup Tables to reduce transaction size.
    /// Pass one element for a single ALT or multiple elements for multi-ALT.
    pub fn address_lookup_table_accounts(mut self, values: Vec<AddressLookupTableAccount>) -> Self {
        self.address_lookup_table_accounts = values;
        self
    }

    /// Use a durable nonce instead of the recent blockhash passed to [`Self::new`].
    ///
    /// This clears `recent_blockhash` because nonce transactions must use the
    /// nonce value as their transaction blockhash.
    pub fn durable_nonce(mut self, value: DurableNonceInfo) -> Self {
        self.durable_nonce = Some(value);
        self.recent_blockhash = None;
        self
    }

    /// Wait for confirmation before returning.
    pub fn wait_tx_confirmed(mut self, value: bool) -> Self {
        self.wait_tx_confirmed = value;
        self
    }

    /// Wait for all SWQoS submit responses and return submitted signatures.
    pub fn wait_for_all_submits(mut self, value: bool) -> Self {
        self.wait_for_all_submits = value;
        self
    }

    /// Simulate instead of submitting.
    pub fn simulate(mut self, value: bool) -> Self {
        self.simulate = value;
        self
    }

    /// Attach upstream receive timestamp for latency tracing.
    pub fn grpc_recv_us(mut self, value: i64) -> Self {
        self.grpc_recv_us = Some(value);
        self
    }
}

impl SimpleSellParams {
    /// Create a simple sell request using a recent blockhash.
    ///
    /// Defaults:
    /// - `account_policy = AccountPolicy::Auto`
    /// - `with_tip = true`
    /// - `wait_tx_confirmed = false`
    /// - `wait_for_all_submits = false`
    /// - `simulate = false`
    /// - no ALT
    /// - no slippage override
    pub fn new(
        dex_type: DexType,
        receive_as: TradeTokenType,
        mint: Pubkey,
        amount: SellAmount,
        extension_params: DexParamEnum,
        recent_blockhash: Hash,
        gas_fee_strategy: GasFeeStrategy,
    ) -> Self {
        Self {
            dex_type,
            receive_as,
            mint,
            amount,
            slippage_basis_points: None,
            recent_blockhash: Some(recent_blockhash),
            extension_params,
            gas_fee_strategy,
            account_policy: AccountPolicy::Auto,
            address_lookup_table_accounts: Vec::new(),
            wait_tx_confirmed: false,
            wait_for_all_submits: false,
            durable_nonce: None,
            simulate: false,
            with_tip: true,
            grpc_recv_us: None,
        }
    }

    /// Create a simple sell request using a durable nonce instead of a recent blockhash.
    pub fn with_durable_nonce(
        dex_type: DexType,
        receive_as: TradeTokenType,
        mint: Pubkey,
        amount: SellAmount,
        extension_params: DexParamEnum,
        durable_nonce: DurableNonceInfo,
        gas_fee_strategy: GasFeeStrategy,
    ) -> Self {
        Self {
            durable_nonce: Some(durable_nonce),
            recent_blockhash: None,
            ..Self::new(
                dex_type,
                receive_as,
                mint,
                amount,
                extension_params,
                Hash::default(),
                gas_fee_strategy,
            )
        }
    }

    /// Set slippage in basis points. `100` means 1%.
    pub fn slippage_basis_points(mut self, value: u64) -> Self {
        self.slippage_basis_points = Some(value);
        self
    }

    /// Set account lifecycle behavior. Bots usually want `HotPathMinimal`;
    /// normal integrations can keep the default `Auto`.
    pub fn account_policy(mut self, value: AccountPolicy) -> Self {
        self.account_policy = value;
        self
    }

    /// Attach Address Lookup Tables to reduce transaction size.
    /// Pass one element for a single ALT or multiple elements for multi-ALT.
    pub fn address_lookup_table_accounts(mut self, values: Vec<AddressLookupTableAccount>) -> Self {
        self.address_lookup_table_accounts = values;
        self
    }

    /// Use a durable nonce instead of the recent blockhash passed to [`Self::new`].
    ///
    /// This clears `recent_blockhash` because nonce transactions must use the
    /// nonce value as their transaction blockhash.
    pub fn durable_nonce(mut self, value: DurableNonceInfo) -> Self {
        self.durable_nonce = Some(value);
        self.recent_blockhash = None;
        self
    }

    /// Wait for confirmation before returning.
    pub fn wait_tx_confirmed(mut self, value: bool) -> Self {
        self.wait_tx_confirmed = value;
        self
    }

    /// Wait for all SWQoS submit responses and return submitted signatures.
    pub fn wait_for_all_submits(mut self, value: bool) -> Self {
        self.wait_for_all_submits = value;
        self
    }

    /// Simulate instead of submitting.
    pub fn simulate(mut self, value: bool) -> Self {
        self.simulate = value;
        self
    }

    /// Enable or disable relay tips for sell transactions.
    pub fn with_tip(mut self, value: bool) -> Self {
        self.with_tip = value;
        self
    }

    /// Attach upstream receive timestamp for latency tracing.
    pub fn grpc_recv_us(mut self, value: i64) -> Self {
        self.grpc_recv_us = Some(value);
        self
    }
}

/// Shared infrastructure components that can be reused across multiple wallets
///
/// This struct holds the expensive-to-initialize components (RPC client, SWQOS clients)
/// that are wallet-independent and can be shared when only the trading wallet changes.
pub struct TradingInfrastructure {
    /// Shared RPC client for blockchain interactions
    pub rpc: Arc<SolanaRpcClient>,
    /// Shared SWQOS clients for transaction priority and routing. Arc<Vec<..>> so cloning into SwapParams is a single Arc clone.
    pub swqos_clients: Arc<Vec<Arc<SwqosClient>>>,
    /// Configuration used to create this infrastructure
    pub config: InfrastructureConfig,
    /// Precomputed at init: min(max SWQOS submit lanes, 2/3 * num_cores). Not computed on trade hot path.
    pub max_sender_concurrency: usize,
    /// Precomputed at init: first max_sender_concurrency CoreIds for job affinity. Empty if no cores. Not computed on trade hot path.
    pub effective_core_ids: Arc<Vec<core_affinity::CoreId>>,
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

        // Initialize rent cache (with timeout so slow RPC doesn't block forever)
        const RENT_UPDATE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
        match tokio::time::timeout(RENT_UPDATE_TIMEOUT, crate::common::seed::update_rents(&rpc))
            .await
        {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                if sdk_log::sdk_log_enabled() {
                    warn!(target: "sol_trade_sdk", "rent update failed: {}, using defaults", e);
                }
                crate::common::seed::set_default_rents();
            }
            Err(_) => {
                if sdk_log::sdk_log_enabled() {
                    warn!(target: "sol_trade_sdk", "rent update timed out ({}s), using defaults; check RPC", RENT_UPDATE_TIMEOUT.as_secs());
                }
                crate::common::seed::set_default_rents();
            }
        }
        crate::common::seed::start_rent_updater(rpc.clone());

        // Create SWQOS clients with blacklist checking（QUIC 握手可能较慢，单节点超时 15s）
        const SWQOS_CLIENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
        let swqos_configs = normalize_swqos_configs(&config.rpc_url, &config.swqos_configs);
        let mut swqos_clients: Vec<Arc<SwqosClient>> = vec![];
        for swqos in &swqos_configs {
            if swqos.is_blacklisted() {
                if sdk_log::sdk_log_enabled() {
                    warn!(target: "sol_trade_sdk", "⚠️ SWQOS {:?} is blacklisted, skipping", swqos.swqos_type());
                }
                continue;
            }
            match tokio::time::timeout(
                SWQOS_CLIENT_TIMEOUT,
                SwqosConfig::get_swqos_client(
                    config.rpc_url.clone(),
                    config.commitment.clone(),
                    swqos.clone(),
                    config.mev_protection,
                ),
            )
            .await
            {
                Ok(Ok(swqos_client)) => swqos_clients.push(swqos_client),
                Ok(Err(err)) => {
                    eprintln!(
                        "⚠️  SWQOS {:?} 初始化失败: {}（已从列表中排除）",
                        swqos.swqos_type(),
                        err
                    );
                    if sdk_log::sdk_log_enabled() {
                        warn!(
                            target: "sol_trade_sdk",
                            "failed to create {:?} swqos client: {err}. Excluding from swqos list",
                            swqos.swqos_type()
                        );
                    }
                }
                Err(_) => {
                    eprintln!(
                        "⚠️  SWQOS {:?} 初始化超时（{}s），已跳过",
                        swqos.swqos_type(),
                        SWQOS_CLIENT_TIMEOUT.as_secs()
                    );
                    if sdk_log::sdk_log_enabled() {
                        warn!(
                            target: "sol_trade_sdk",
                            "swqos {:?} init timed out ({}s), skipping",
                            swqos.swqos_type(),
                            SWQOS_CLIENT_TIMEOUT.as_secs()
                        );
                    }
                }
            }
        }

        // 若全部失败、被黑名单跳过或仅配置了不可用通道，至少保留一条 Rpc Default，否则 execute_parallel 会因 swqos_clients 为空直接报错。
        if swqos_clients.is_empty() {
            eprintln!(
                "⚠️  无任何 SWQOS 客户端初始化成功，将回退为普通 RPC 发送: {}",
                config.rpc_url
            );
            if sdk_log::sdk_log_enabled() {
                warn!(
                    target: "sol_trade_sdk",
                    "no SWQOS clients initialized; falling back to Rpc Default ({})",
                    config.rpc_url
                );
            }
            match SwqosConfig::get_swqos_client(
                config.rpc_url.clone(),
                config.commitment.clone(),
                SwqosConfig::Default(config.rpc_url.clone()),
                config.mev_protection,
            )
            .await
            {
                Ok(c) => swqos_clients.push(c),
                Err(e) => {
                    if sdk_log::sdk_log_enabled() {
                        warn!(
                            target: "sol_trade_sdk",
                            "fallback Rpc Default client failed: {}",
                            e
                        );
                    }
                }
            }
        }

        if !swqos_clients.is_empty() {
            let labels: Vec<&str> =
                swqos_clients.iter().map(|c| c.get_swqos_type().as_str()).collect();
            eprintln!("ℹ️  SWQOS 通道已就绪: {} 条 → [{}]", swqos_clients.len(), labels.join(", "));
        }

        let max_submit_lanes = swqos_clients
            .iter()
            .map(|client| {
                if matches!(client.get_swqos_type(), SwqosType::Default) {
                    1usize
                } else {
                    2usize
                }
            })
            .sum::<usize>()
            .max(1);
        let (max_sender_concurrency, effective_core_ids) = {
            let num_cores = core_affinity::get_core_ids().map(|c| c.len()).unwrap_or(0);
            let max_by_cores = (num_cores * 2 / 3).max(1);
            let cap = max_submit_lanes.min(max_by_cores).max(1);
            let ids = core_affinity::get_core_ids()
                .map(|all| {
                    let v: Vec<_> = all.into_iter().collect();
                    let len = v.len();
                    if config.swqos_cores_from_end && len >= cap {
                        v.into_iter().skip(len - cap).collect()
                    } else {
                        v.into_iter().take(cap).collect()
                    }
                })
                .unwrap_or_default();
            (cap, Arc::new(ids))
        };

        crate::instruction::utils::pumpswap::warm_pumpswap_global_config(Some(&rpc)).await;

        Self {
            rpc,
            swqos_clients: Arc::new(swqos_clients),
            config,
            max_sender_concurrency,
            effective_core_ids,
        }
    }
}

/// When using `TradeConfig::with_swqos_cores_from_end(true)`, returns the same "last N" core indices
/// that the infrastructure uses. Pass the result to `TradingClient::with_dedicated_sender_threads`
/// for 方式 C (组合使用): SWQOS on last N cores and dedicated sender threads pinned to those cores.
///
/// Returns `None` if core count cannot be determined. `swqos_count` is typically `swqos_configs.len()`.
pub fn recommended_sender_thread_core_indices(swqos_count: usize) -> Option<Vec<usize>> {
    let all = core_affinity::get_core_ids()?;
    let num_cores = all.len();
    if num_cores == 0 {
        return None;
    }
    let max_by_cores = (num_cores * 2 / 3).max(1);
    let cap = swqos_count.min(max_by_cores).max(1).min(num_cores);
    let start = num_cores.saturating_sub(cap);
    Some((start..num_cores).collect())
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
    /// Internal: use dedicated sender threads (default false). Set via with_dedicated_sender_threads() for advanced use.
    pub use_dedicated_sender_threads: bool,
    /// Internal: core indices for dedicated sender threads. Trimmed to ≤ max_sender_concurrency at set.
    pub sender_thread_cores: Option<Arc<Vec<usize>>>,
    /// Internal: precomputed at infra init (min(swqos_count, 2/3*cores)). Not user-configurable.
    pub max_sender_concurrency: usize,
    /// Internal: precomputed at infra init for job affinity. Not user-configurable.
    pub effective_core_ids: Arc<Vec<core_affinity::CoreId>>,
    /// Whether to output all SDK logs (from TradeConfig.log_enabled).
    pub log_enabled: bool,
    /// Whether to check minimum tip per SWQOS (from TradeConfig.check_min_tip). Default false for lower latency.
    pub check_min_tip: bool,
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
            use_dedicated_sender_threads: self.use_dedicated_sender_threads,
            sender_thread_cores: self.sender_thread_cores.clone(),
            max_sender_concurrency: self.max_sender_concurrency,
            effective_core_ids: self.effective_core_ids.clone(),
            log_enabled: self.log_enabled,
            check_min_tip: self.check_min_tip,
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
    /// Optional address lookup tables for transaction size optimization.
    /// Pass one element for a single ALT or multiple elements for multi-ALT.
    pub address_lookup_table_accounts: Vec<AddressLookupTableAccount>,
    /// Whether to wait for transaction confirmation before returning
    pub wait_tx_confirmed: bool,
    /// When true, wait for every SWQOS route's HTTP submit response so all
    /// submitted signatures are returned. This applies whether SDK confirmation
    /// is enabled or the caller monitors externally. Recent-blockhash route
    /// variants are not mutually exclusive; durable nonce variants are.
    /// Defaults to false. See `SwapParams.wait_for_all_submits`.
    pub wait_for_all_submits: bool,
    /// Whether to create input token associated token account
    pub create_input_token_ata: bool,
    /// Whether to close input token associated token account after trade
    pub close_input_token_ata: bool,
    /// Whether to create token mint associated token account
    pub create_mint_ata: bool,
    /// Durable nonce information
    pub durable_nonce: Option<DurableNonceInfo>,
    /// Optional fixed output token amount. On exact-out capable DEXes this uses
    /// the exact-out instruction and treats `input_token_amount` as max input.
    pub fixed_output_token_amount: Option<u64>,
    /// Gas fee strategy
    pub gas_fee_strategy: GasFeeStrategy,
    /// Whether to simulate the transaction instead of executing it
    pub simulate: bool,
    /// Use exact quote-input buy instructions (legacy PumpFun uses SOL quote; V2/PumpSwap use generic quote).
    /// When Some(true) or None (default), the exact SOL/quote amount is spent and slippage is applied to output tokens.
    /// When Some(false), uses regular buy instruction where slippage is applied to SOL/quote input.
    /// This option only applies to PumpFun and PumpSwap DEXes; it is ignored for other DEXes.
    pub use_exact_sol_amount: Option<bool>,
    /// Optional upstream receive timestamp (e.g. gRPC recv) in microseconds for latency tracing.
    pub grpc_recv_us: Option<i64>,
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
    /// Optional address lookup tables for transaction size optimization.
    /// Pass one element for a single ALT or multiple elements for multi-ALT.
    pub address_lookup_table_accounts: Vec<AddressLookupTableAccount>,
    /// Whether to wait for transaction confirmation before returning
    pub wait_tx_confirmed: bool,
    /// When true, wait for every SWQOS route's HTTP submit response so all
    /// submitted signatures are returned. This applies whether SDK confirmation
    /// is enabled or the caller monitors externally. Recent-blockhash route
    /// variants are not mutually exclusive; durable nonce variants are.
    /// Defaults to false. See `SwapParams.wait_for_all_submits`.
    pub wait_for_all_submits: bool,
    /// Whether to create output token associated token account
    pub create_output_token_ata: bool,
    /// Whether to close output token associated token account after trade
    pub close_output_token_ata: bool,
    /// Whether to close mint token associated token account after trade
    pub close_mint_token_ata: bool,
    /// Durable nonce information
    pub durable_nonce: Option<DurableNonceInfo>,
    /// Optional fixed output token amount. On exact-out capable DEXes this uses
    /// the exact-out instruction and treats `input_token_amount` as max input.
    pub fixed_output_token_amount: Option<u64>,
    /// Gas fee strategy
    pub gas_fee_strategy: GasFeeStrategy,
    /// Whether to simulate the transaction instead of executing it
    pub simulate: bool,
    /// Optional upstream receive timestamp (e.g. gRPC recv) in microseconds for latency tracing.
    pub grpc_recv_us: Option<i64>,
}

#[inline]
fn buy_account_flags(policy: AccountPolicy) -> (bool, bool, bool) {
    match policy {
        AccountPolicy::Auto => (false, true, false),
        AccountPolicy::HotPathMinimal | AccountPolicy::AssumePrepared => (false, false, false),
        AccountPolicy::CreateMissing => (true, true, false),
    }
}

#[inline]
fn sell_account_flags(policy: AccountPolicy, receive_as: &TradeTokenType) -> (bool, bool, bool) {
    match policy {
        AccountPolicy::Auto => (*receive_as != TradeTokenType::SOL, false, false),
        AccountPolicy::HotPathMinimal | AccountPolicy::AssumePrepared => (false, false, false),
        AccountPolicy::CreateMissing => (true, false, false),
    }
}

impl From<SimpleBuyParams> for TradeBuyParams {
    fn from(params: SimpleBuyParams) -> Self {
        let (input_token_amount, fixed_output_token_amount, use_exact_sol_amount) =
            match params.amount {
                BuyAmount::ExactInput(amount) => (amount, None, Some(true)),
                BuyAmount::ExactOutput { output_amount, max_input_amount } => {
                    (max_input_amount, Some(output_amount), Some(true))
                }
                BuyAmount::WithMaxInput { quote_amount } => (quote_amount, None, Some(false)),
            };
        let (create_input_token_ata, create_mint_ata, close_input_token_ata) =
            buy_account_flags(params.account_policy);

        TradeBuyParams {
            dex_type: params.dex_type,
            input_token_type: params.pay_with,
            mint: params.mint,
            input_token_amount,
            slippage_basis_points: params.slippage_basis_points,
            recent_blockhash: params.recent_blockhash,
            extension_params: params.extension_params,
            address_lookup_table_accounts: params.address_lookup_table_accounts,
            wait_tx_confirmed: params.wait_tx_confirmed,
            wait_for_all_submits: params.wait_for_all_submits,
            create_input_token_ata,
            close_input_token_ata,
            create_mint_ata,
            durable_nonce: params.durable_nonce,
            fixed_output_token_amount,
            gas_fee_strategy: params.gas_fee_strategy,
            simulate: params.simulate,
            use_exact_sol_amount,
            grpc_recv_us: params.grpc_recv_us,
        }
    }
}

impl From<SimpleSellParams> for TradeSellParams {
    fn from(params: SimpleSellParams) -> Self {
        let (input_token_amount, fixed_output_token_amount) = match params.amount {
            SellAmount::ExactInput(amount) => (amount, None),
            SellAmount::ExactOutput { output_amount, max_input_amount } => {
                (max_input_amount, Some(output_amount))
            }
        };
        let (create_output_token_ata, close_output_token_ata, close_mint_token_ata) =
            sell_account_flags(params.account_policy, &params.receive_as);

        TradeSellParams {
            dex_type: params.dex_type,
            output_token_type: params.receive_as,
            mint: params.mint,
            input_token_amount,
            slippage_basis_points: params.slippage_basis_points,
            recent_blockhash: params.recent_blockhash,
            with_tip: params.with_tip,
            extension_params: params.extension_params,
            address_lookup_table_accounts: params.address_lookup_table_accounts,
            wait_tx_confirmed: params.wait_tx_confirmed,
            wait_for_all_submits: params.wait_for_all_submits,
            create_output_token_ata,
            close_output_token_ata,
            close_mint_token_ata,
            durable_nonce: params.durable_nonce,
            fixed_output_token_amount,
            gas_fee_strategy: params.gas_fee_strategy,
            simulate: params.simulate,
            grpc_recv_us: params.grpc_recv_us,
        }
    }
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
        let max_sender_concurrency = infrastructure.max_sender_concurrency;
        let effective_core_ids = infrastructure.effective_core_ids.clone();

        Self {
            payer,
            infrastructure,
            middleware_manager: None,
            use_seed_optimize,
            use_dedicated_sender_threads: false,
            sender_thread_cores: None,
            max_sender_concurrency,
            effective_core_ids,
            log_enabled: true,
            check_min_tip: false,
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
            // 在后台异步创建 WSOL ATA，不阻塞启动
            let payer_clone = payer.clone();
            let rpc_clone = infrastructure.rpc.clone();
            tokio::spawn(async move {
                Self::ensure_wsol_ata(&payer_clone, &rpc_clone).await;
            });
            if sdk_log::sdk_log_enabled() {
                info!(target: "sol_trade_sdk", "ℹ️ WSOL ATA creation started in background, does not block bot startup");
            }
        }

        let max_sender_concurrency = infrastructure.max_sender_concurrency;
        let effective_core_ids = infrastructure.effective_core_ids.clone();

        Self {
            payer,
            infrastructure,
            middleware_manager: None,
            use_seed_optimize,
            use_dedicated_sender_threads: false,
            sender_thread_cores: None,
            max_sender_concurrency,
            effective_core_ids,
            log_enabled: true,
            check_min_tip: false,
        }
    }

    /// 单次尝试创建 WSOL ATA：获取 blockhash、组交易、发送并确认。成功或账户已存在返回 Ok(())，否则返回 Err(错误信息)。
    async fn try_create_wsol_ata_once(
        rpc: &SolanaRpcClient,
        payer: &Arc<Keypair>,
        wsol_ata: &solana_sdk::pubkey::Pubkey,
        create_ata_ixs: &[solana_sdk::instruction::Instruction],
        timeout_secs: u64,
    ) -> Result<(), String> {
        use solana_sdk::transaction::Transaction;
        let recent_blockhash = rpc
            .get_latest_blockhash()
            .await
            .map_err(|e| format!("Failed to get blockhash: {}", e))?;
        let tx = Transaction::new_signed_with_payer(
            create_ata_ixs,
            Some(&payer.pubkey()),
            &[payer.as_ref()],
            recent_blockhash,
        );
        let send_result = tokio::time::timeout(
            tokio::time::Duration::from_secs(timeout_secs),
            rpc.send_and_confirm_transaction(&tx),
        )
        .await;
        match send_result {
            Ok(Ok(_signature)) => Ok(()),
            Ok(Err(e)) => {
                if rpc.get_account(wsol_ata).await.is_ok() {
                    return Ok(());
                }
                Err(format!("{}", e))
            }
            Err(_) => Err(format!("Transaction confirmation timeout ({}s)", timeout_secs)),
        }
    }

    /// 确保钱包存在 WSOL ATA；不存在则发交易创建（会花费租金 + 手续费，初始化阶段唯一会扣钱的逻辑）
    async fn ensure_wsol_ata(payer: &Arc<Keypair>, rpc: &Arc<SolanaRpcClient>) {
        const MAX_RETRIES: usize = 3;
        const TIMEOUT_SECS: u64 = 10;

        let wsol_ata = crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &payer.pubkey(),
            &WSOL_TOKEN_ACCOUNT,
            &crate::constants::TOKEN_PROGRAM,
        );

        if rpc.get_account(&wsol_ata).await.is_ok() {
            if sdk_log::sdk_log_enabled() {
                info!(target: "sol_trade_sdk", "✅ WSOL ATA already exists: {}", wsol_ata);
            }
            return;
        }

        let create_ata_ixs = crate::trading::common::wsol_manager::create_wsol_ata(&payer.pubkey());
        if create_ata_ixs.is_empty() {
            if sdk_log::sdk_log_enabled() {
                info!(target: "sol_trade_sdk", "ℹ️ WSOL ATA already exists (no need to create)");
            }
            return;
        }

        if sdk_log::sdk_log_enabled() {
            info!(target: "sol_trade_sdk", "🔨 Creating WSOL ATA: {}", wsol_ata);
        }
        let mut last_error = None;
        for attempt in 1..=MAX_RETRIES {
            if attempt > 1 {
                if sdk_log::sdk_log_enabled() {
                    info!(target: "sol_trade_sdk", "🔄 Retrying WSOL ATA creation (attempt {}/{})...", attempt, MAX_RETRIES);
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            }
            match Self::try_create_wsol_ata_once(
                rpc.as_ref(),
                payer,
                &wsol_ata,
                &create_ata_ixs,
                TIMEOUT_SECS,
            )
            .await
            {
                Ok(()) => {
                    if sdk_log::sdk_log_enabled() {
                        info!(target: "sol_trade_sdk", "✅ WSOL ATA created or already exists");
                    }
                    return;
                }
                Err(e) => {
                    last_error = Some(e.clone());
                    if attempt < MAX_RETRIES && sdk_log::sdk_log_enabled() {
                        warn!(target: "sol_trade_sdk", "⚠️ Attempt {} failed: {}", attempt, e);
                    }
                }
            }
        }

        if let Some(err) = last_error {
            if sdk_log::sdk_log_enabled() {
                error!(target: "sol_trade_sdk", "❌ WSOL ATA creation failed after {} retries: {}", MAX_RETRIES, wsol_ata);
                error!(target: "sol_trade_sdk", "   Error: {}", err);
                error!(target: "sol_trade_sdk", "   💡 Possible causes: insufficient SOL, RPC timeout, or fee");
                error!(target: "sol_trade_sdk", "   🔧 Solutions: fund wallet (e.g. 0.1 SOL), retry, check RPC");
            }
            std::thread::sleep(std::time::Duration::from_secs(5));
            panic!(
                "❌ WSOL ATA creation failed and account does not exist: {}. Error: {}",
                wsol_ata, err
            );
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
        // 设置 SDK 全局日志开关，后续所有 SDK 内日志（SWQOS/WSOL/耗时等）均受此控制
        sdk_log::set_sdk_log_enabled(trade_config.log_enabled);
        // 预热高性能时钟，避免首笔交易时触发 3 次 Utc::now() 校准
        let _ = crate::common::clock::now_micros();
        // Create infrastructure from trade config
        let infra_config = InfrastructureConfig::from_trade_config(&trade_config);
        let infrastructure = Arc::new(TradingInfrastructure::new(infra_config).await);

        // Initialize wallet-specific caches
        crate::common::fast_fn::fast_init(&payer.pubkey());

        // ═══════════════════════════════════════════════════════════════════════════════
        // 初始化阶段会花费租金/手续费的唯一路径：创建 WSOL ATA（ensure_wsol_ata）
        // - 触发条件：create_wsol_ata_on_startup == true 且钱包 SOL >= MIN_SOL_FOR_WSOL_ATA_LAMPORTS
        // - 花费：ATA 租金（约 0.00203928 SOL）+ 交易手续费；钱包不足时已跳过
        // - 其它初始化（TradingInfrastructure::new、update_rents、get_swqos_client）仅 RPC/HTTP，不发送交易
        // ═══════════════════════════════════════════════════════════════════════════════
        if trade_config.create_wsol_ata_on_startup {
            const MIN_SOL_FOR_WSOL_ATA_LAMPORTS: u64 = 500_000; // 约 0.0005 SOL，用于 ATA 租金 + 手续费
            const BALANCE_CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
            let balance = tokio::time::timeout(
                BALANCE_CHECK_TIMEOUT,
                infrastructure.rpc.get_balance(&payer.pubkey()),
            )
            .await
            .unwrap_or(Ok(0))
            .unwrap_or(0);
            if balance >= MIN_SOL_FOR_WSOL_ATA_LAMPORTS {
                Self::ensure_wsol_ata(&payer, &infrastructure.rpc).await;
            } else if sdk_log::sdk_log_enabled() {
                info!(
                    target: "sol_trade_sdk",
                    "⏭️ 跳过创建 WSOL ATA：钱包 SOL 不足（当前 {} lamports，需要至少 {}）",
                    balance,
                    MIN_SOL_FOR_WSOL_ATA_LAMPORTS
                );
            }
        }

        // 并发/核心相关由 infrastructure 预计算，用户无需配置
        let instance = Self {
            payer,
            infrastructure: infrastructure.clone(),
            middleware_manager: None,
            use_seed_optimize: trade_config.use_seed_optimize,
            use_dedicated_sender_threads: false,
            sender_thread_cores: None,
            max_sender_concurrency: infrastructure.max_sender_concurrency,
            effective_core_ids: infrastructure.effective_core_ids.clone(),
            log_enabled: trade_config.log_enabled,
            check_min_tip: trade_config.check_min_tip,
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

    /// **Advanced.** Use dedicated OS threads for sender pool (and optionally pin to cores).  
    /// By default the SDK uses a shared tokio pool; this can reduce scheduling contention when sending many txs.  
    /// Concurrency and core count are capped internally (≤ max submit lanes, ≤ 2/3 of CPU cores).
    /// - `None`: keep default (shared tokio pool).  
    /// - `Some(vec![])`: dedicated threads with default count, no core pinning.  
    /// - `Some(indices)`: dedicated threads pinned to those core indices (trimmed to cap).  
    ///  
    /// **Latency note:** If a core is busy with other work (node, bot), SWQOS submit on that core can be delayed.  
    /// For lowest latency, pass core indices that are *reserved* for SWQOS (do not run other CPU-heavy work on those cores).
    pub fn with_dedicated_sender_threads(mut self, core_indices: Option<Vec<usize>>) -> Self {
        match core_indices {
            None => {
                self.use_dedicated_sender_threads = false;
                self.sender_thread_cores = None;
            }
            Some(v) if v.is_empty() => {
                self.use_dedicated_sender_threads = true;
                self.sender_thread_cores = None;
            }
            Some(v) => {
                self.use_dedicated_sender_threads = true;
                let cap = v.len().min(self.max_sender_concurrency);
                self.sender_thread_cores =
                    Some(Arc::new(if cap < v.len() { v[..cap].to_vec() } else { v }));
            }
        }
        if self.use_dedicated_sender_threads {
            crate::trading::core::async_executor::warm_dedicated_sender_pool(
                self.sender_thread_cores.as_ref().map(|v| v.as_slice()),
                self.max_sender_concurrency,
            );
        }
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
    ) -> Result<
        (bool, Vec<Signature>, Option<TradeError>, Vec<(crate::swqos::SwqosType, i64)>),
        anyhow::Error,
    > {
        validate_trade_safety(
            "buy",
            params.input_token_amount,
            params.fixed_output_token_amount,
            params.slippage_basis_points,
        )?;
        if params.recent_blockhash.is_none() && params.durable_nonce.is_none() {
            return Err(anyhow::anyhow!(
                "Must provide either recent_blockhash or durable_nonce for buy (required for transaction validity)"
            ));
        }
        #[cfg(feature = "perf-trace")]
        if sdk_log::sdk_log_enabled() && params.slippage_basis_points.is_none() {
            debug!(
                target: "sol_trade_sdk",
                "slippage_basis_points is none, use default slippage basis points: {}",
                DEFAULT_SLIPPAGE
            );
        }
        if params.input_token_type == TradeTokenType::USD1 && params.dex_type != DexType::Bonk {
            return Err(anyhow::anyhow!(
                " Current version only supports USD1 trading on Bonk protocols"
            ));
        }
        let protocol_params = params.extension_params;
        if !validate_protocol_params(params.dex_type, &protocol_params) {
            return Err(anyhow::anyhow!(
                "Invalid protocol params for Trade (dex={:?})",
                params.dex_type
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
        let executor = TradeFactory::create_executor(params.dex_type);
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
            address_lookup_table_accounts: params.address_lookup_table_accounts,
            recent_blockhash: params.recent_blockhash,
            wait_tx_confirmed: params.wait_tx_confirmed,
            protocol_params,
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
            log_enabled: self.log_enabled,
            wait_for_all_submits: params.wait_for_all_submits,
            use_dedicated_sender_threads: self.use_dedicated_sender_threads,
            sender_thread_cores: self.sender_thread_cores.clone(),
            max_sender_concurrency: self.max_sender_concurrency,
            effective_core_ids: self.effective_core_ids.clone(),
            check_min_tip: self.check_min_tip,
            grpc_recv_us: params.grpc_recv_us,
            use_exact_sol_amount: params.use_exact_sol_amount,
        };

        let swap_result = executor.swap(buy_params).await;
        let result = swap_result.map(|(success, sigs, err, timings)| {
            let legacy_timings = timings
                .into_iter()
                .map(|timing| (timing.swqos_type, timing.submit_done_us))
                .collect();
            (success, sigs, err.map(TradeError::from), legacy_timings)
        });
        result
    }

    /// Execute a high-level buy request.
    #[inline]
    pub async fn buy_simple(
        &self,
        params: SimpleBuyParams,
    ) -> Result<
        (bool, Vec<Signature>, Option<TradeError>, Vec<(crate::swqos::SwqosType, i64)>),
        anyhow::Error,
    > {
        self.buy(params.into()).await
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
    ) -> Result<
        (bool, Vec<Signature>, Option<TradeError>, Vec<(crate::swqos::SwqosType, i64)>),
        anyhow::Error,
    > {
        validate_trade_safety(
            "sell",
            params.input_token_amount,
            params.fixed_output_token_amount,
            params.slippage_basis_points,
        )?;
        #[cfg(feature = "perf-trace")]
        if sdk_log::sdk_log_enabled() && params.slippage_basis_points.is_none() {
            debug!(
                target: "sol_trade_sdk",
                "slippage_basis_points is none, use default slippage basis points: {}",
                DEFAULT_SLIPPAGE
            );
        }
        if params.recent_blockhash.is_none() && params.durable_nonce.is_none() {
            return Err(anyhow::anyhow!(
                "Must provide either recent_blockhash or durable_nonce for sell (required for transaction validity)"
            ));
        }
        if params.output_token_type == TradeTokenType::USD1 && params.dex_type != DexType::Bonk {
            return Err(anyhow::anyhow!(
                " Current version only supports USD1 trading on Bonk protocols"
            ));
        }
        let protocol_params = params.extension_params;
        if !validate_protocol_params(params.dex_type, &protocol_params) {
            return Err(anyhow::anyhow!(
                "Invalid protocol params for Trade (dex={:?})",
                params.dex_type
            ));
        }
        let executor = TradeFactory::create_executor(params.dex_type);
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
            address_lookup_table_accounts: params.address_lookup_table_accounts,
            recent_blockhash: params.recent_blockhash,
            wait_tx_confirmed: params.wait_tx_confirmed,
            protocol_params,
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
            log_enabled: self.log_enabled,
            wait_for_all_submits: params.wait_for_all_submits,
            use_dedicated_sender_threads: self.use_dedicated_sender_threads,
            sender_thread_cores: self.sender_thread_cores.clone(),
            max_sender_concurrency: self.max_sender_concurrency,
            effective_core_ids: self.effective_core_ids.clone(),
            check_min_tip: self.check_min_tip,
            grpc_recv_us: params.grpc_recv_us,
            use_exact_sol_amount: None,
        };

        let swap_result = executor.swap(sell_params).await;
        let result = swap_result.map(|(success, sigs, err, timings)| {
            let legacy_timings = timings
                .into_iter()
                .map(|timing| (timing.swqos_type, timing.submit_done_us))
                .collect();
            (success, sigs, err.map(TradeError::from), legacy_timings)
        });
        result
    }

    /// Execute a high-level sell request.
    #[inline]
    pub async fn sell_simple(
        &self,
        params: SimpleSellParams,
    ) -> Result<
        (bool, Vec<Signature>, Option<TradeError>, Vec<(crate::swqos::SwqosType, i64)>),
        anyhow::Error,
    > {
        self.sell(params.into()).await
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
    ) -> Result<
        (bool, Vec<Signature>, Option<TradeError>, Vec<(crate::swqos::SwqosType, i64)>),
        anyhow::Error,
    > {
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
        use crate::common::seed::get_associated_token_address_with_program_id_use_seed;
        use crate::trading::common::wsol_manager::{
            wrap_wsol_to_sol as wrap_wsol_to_sol_internal, wrap_wsol_to_sol_without_create,
        };
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
        let mut transaction =
            Transaction::new_with_payer(&instructions, Some(&self.payer.pubkey()));
        transaction.sign(&[&*self.payer], recent_blockhash);
        let signature = self.infrastructure.rpc.send_and_confirm_transaction(&transaction).await?;
        Ok(signature.to_string())
    }

    /// Claim Bonding Curve (Pump) cashback.
    ///
    /// Transfers native SOL from the user's UserVolumeAccumulator to the wallet.
    /// If there is nothing to claim, the transaction may still succeed with no SOL transferred.
    ///
    /// # Returns
    /// * `Ok(String)` - Transaction signature
    /// * `Err(anyhow::Error)` - Build or send failure (e.g. invalid PDA)
    pub async fn claim_cashback_pumpfun(&self) -> Result<String, anyhow::Error> {
        use solana_sdk::transaction::Transaction;
        let ix = crate::instruction::pumpfun::claim_cashback_pumpfun_instruction(
            &self.payer.pubkey(),
        )
        .ok_or_else(|| anyhow::anyhow!("Failed to build PumpFun claim_cashback instruction"))?;
        let recent_blockhash = self.infrastructure.rpc.get_latest_blockhash().await?;
        let mut transaction = Transaction::new_with_payer(&[ix], Some(&self.payer.pubkey()));
        transaction.sign(&[&*self.payer], recent_blockhash);
        let signature = self.infrastructure.rpc.send_and_confirm_transaction(&transaction).await?;
        Ok(signature.to_string())
    }

    /// Claim PumpSwap (AMM) cashback.
    ///
    /// Transfers WSOL from the UserVolumeAccumulator to the user's WSOL ATA.
    /// Creates the user's WSOL ATA idempotently if it does not exist, then claims.
    ///
    /// # Returns
    /// * `Ok(String)` - Transaction signature
    /// * `Err(anyhow::Error)` - Build or send failure
    pub async fn claim_cashback_pumpswap(&self) -> Result<String, anyhow::Error> {
        use solana_sdk::transaction::Transaction;
        let mut instructions =
            crate::common::fast_fn::create_associated_token_account_idempotent_fast_use_seed(
                &self.payer.pubkey(),
                &self.payer.pubkey(),
                &WSOL_TOKEN_ACCOUNT,
                &crate::constants::TOKEN_PROGRAM,
                self.use_seed_optimize,
            );
        let ix = crate::instruction::pumpswap::claim_cashback_pumpswap_instruction(
            &self.payer.pubkey(),
            WSOL_TOKEN_ACCOUNT,
            crate::constants::TOKEN_PROGRAM,
        )
        .ok_or_else(|| anyhow::anyhow!("Failed to build PumpSwap claim_cashback instruction"))?;
        instructions.push(ix);
        let recent_blockhash = self.infrastructure.rpc.get_latest_blockhash().await?;
        let mut transaction =
            Transaction::new_with_payer(&instructions, Some(&self.payer.pubkey()));
        transaction.sign(&[&*self.payer], recent_blockhash);
        let signature = self.infrastructure.rpc.send_and_confirm_transaction(&transaction).await?;
        Ok(signature.to_string())
    }
}

fn validate_trade_safety(
    side: &str,
    input_amount: u64,
    fixed_output_amount: Option<u64>,
    slippage_basis_points: Option<u64>,
) -> Result<(), anyhow::Error> {
    if input_amount == 0 {
        return Err(anyhow::anyhow!("{} input amount must be greater than zero", side));
    }
    if fixed_output_amount == Some(0) {
        return Err(anyhow::anyhow!("{} fixed output amount must be greater than zero", side));
    }
    if let Some(bps) = slippage_basis_points {
        if bps >= 10_000 {
            return Err(anyhow::anyhow!(
                "{} slippage_basis_points must be below 10000, got {}",
                side,
                bps
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instruction::utils::pumpfun::global_constants;
    use crate::swqos::SwqosRegion;
    use std::sync::Arc;

    fn dummy_pumpfun_params() -> DexParamEnum {
        DexParamEnum::PumpFun(PumpFunParams {
            bonding_curve: Arc::new(Default::default()),
            associated_bonding_curve: Pubkey::default(),
            observed_trade_creator: None,
            creator_vault: Pubkey::default(),
            fee_sharing_creator_vault_if_active: None,
            token_program: Pubkey::default(),
            close_token_account_when_sell: None,
            fee_recipient: global_constants::FEE_RECIPIENT,
            quote_mint: Pubkey::default(),
        })
    }

    #[test]
    fn trade_safety_rejects_zero_amounts_and_unbounded_slippage() {
        assert!(validate_trade_safety("buy", 0, None, Some(100)).is_err());
        assert!(validate_trade_safety("buy", 1, Some(0), Some(100)).is_err());
        assert!(validate_trade_safety("sell", 1, None, Some(10_000)).is_err());
        assert!(validate_trade_safety("sell", 1, None, Some(u64::MAX)).is_err());
    }

    #[test]
    fn trade_safety_accepts_bounded_values() {
        assert!(validate_trade_safety("buy", 1, None, None).is_ok());
        assert!(validate_trade_safety("buy", 1, Some(1), Some(9_999)).is_ok());
    }

    #[test]
    fn normalize_swqos_configs_adds_default_rpc_route() {
        let configs = vec![SwqosConfig::Jito("uuid".to_string(), SwqosRegion::Frankfurt, None)];
        let normalized = normalize_swqos_configs("https://rpc.example", &configs);

        assert_eq!(normalized.len(), 2);
        assert!(normalized.iter().any(|c| matches!(c.swqos_type(), SwqosType::Jito)));
        assert!(normalized.iter().any(|c| matches!(c.swqos_type(), SwqosType::Default)));
    }

    #[test]
    fn normalize_swqos_configs_does_not_duplicate_default_rpc_route() {
        let configs = vec![SwqosConfig::Default("https://rpc.example".to_string())];
        let normalized = normalize_swqos_configs("https://rpc.example", &configs);

        assert_eq!(normalized.len(), 1);
        assert!(matches!(normalized[0].swqos_type(), SwqosType::Default));
    }

    #[test]
    fn simple_buy_hot_path_maps_to_low_level_params() {
        let simple = SimpleBuyParams {
            dex_type: DexType::PumpFun,
            pay_with: TradeTokenType::SOL,
            mint: Pubkey::new_unique(),
            amount: BuyAmount::WithMaxInput { quote_amount: 10_000 },
            slippage_basis_points: Some(100),
            recent_blockhash: Some(Hash::new_unique()),
            extension_params: dummy_pumpfun_params(),
            gas_fee_strategy: GasFeeStrategy::new(),
            account_policy: AccountPolicy::HotPathMinimal,
            address_lookup_table_accounts: Vec::new(),
            wait_tx_confirmed: false,
            wait_for_all_submits: false,
            durable_nonce: None,
            simulate: false,
            grpc_recv_us: None,
        };

        let low: TradeBuyParams = simple.into();

        assert!(matches!(low.input_token_type, TradeTokenType::SOL));
        assert_eq!(low.input_token_amount, 10_000);
        assert_eq!(low.use_exact_sol_amount, Some(false));
        assert_eq!(low.fixed_output_token_amount, None);
        assert!(!low.create_input_token_ata);
        assert!(!low.close_input_token_ata);
        assert!(!low.create_mint_ata);
    }

    #[test]
    fn simple_buy_auto_creates_target_mint_ata() {
        let simple = SimpleBuyParams {
            dex_type: DexType::PumpFun,
            pay_with: TradeTokenType::SOL,
            mint: Pubkey::new_unique(),
            amount: BuyAmount::ExactOutput { output_amount: 42, max_input_amount: 10_000 },
            slippage_basis_points: None,
            recent_blockhash: Some(Hash::new_unique()),
            extension_params: dummy_pumpfun_params(),
            gas_fee_strategy: GasFeeStrategy::new(),
            account_policy: AccountPolicy::Auto,
            address_lookup_table_accounts: Vec::new(),
            wait_tx_confirmed: false,
            wait_for_all_submits: false,
            durable_nonce: None,
            simulate: false,
            grpc_recv_us: None,
        };

        let low: TradeBuyParams = simple.into();

        assert_eq!(low.input_token_amount, 10_000);
        assert_eq!(low.fixed_output_token_amount, Some(42));
        assert_eq!(low.use_exact_sol_amount, Some(true));
        assert!(low.create_mint_ata);
        assert!(!low.create_input_token_ata);
    }

    #[test]
    fn simple_buy_builder_sets_defaults_and_overrides() {
        let simple = SimpleBuyParams::new(
            DexType::PumpFun,
            TradeTokenType::SOL,
            Pubkey::new_unique(),
            BuyAmount::ExactInput(10_000),
            dummy_pumpfun_params(),
            Hash::new_unique(),
            GasFeeStrategy::new(),
        )
        .slippage_basis_points(250)
        .account_policy(AccountPolicy::HotPathMinimal);

        let low: TradeBuyParams = simple.into();

        assert_eq!(low.slippage_basis_points, Some(250));
        assert_eq!(low.use_exact_sol_amount, Some(true));
        assert!(!low.create_input_token_ata);
        assert!(!low.create_mint_ata);
        assert!(!low.wait_tx_confirmed);
    }

    #[test]
    fn simple_buy_builder_maps_multiple_lookup_tables() {
        let alt1 = AddressLookupTableAccount {
            key: Pubkey::new_unique(),
            addresses: vec![Pubkey::new_unique()],
        };
        let alt2 = AddressLookupTableAccount {
            key: Pubkey::new_unique(),
            addresses: vec![Pubkey::new_unique()],
        };

        let simple = SimpleBuyParams::new(
            DexType::PumpFun,
            TradeTokenType::SOL,
            Pubkey::new_unique(),
            BuyAmount::ExactInput(10_000),
            dummy_pumpfun_params(),
            Hash::new_unique(),
            GasFeeStrategy::new(),
        )
        .address_lookup_table_accounts(vec![alt1.clone(), alt2.clone()]);

        let low: TradeBuyParams = simple.into();

        assert_eq!(low.address_lookup_table_accounts.len(), 2);
        assert_eq!(low.address_lookup_table_accounts[0].key, alt1.key);
        assert_eq!(low.address_lookup_table_accounts[1].key, alt2.key);
    }

    #[test]
    fn simple_buy_builder_can_use_durable_nonce() {
        let nonce_account = Pubkey::new_unique();
        let nonce_hash = Hash::new_unique();
        let durable_nonce = DurableNonceInfo {
            nonce_account: Some(nonce_account),
            current_nonce: Some(nonce_hash),
        };

        let simple = SimpleBuyParams::new(
            DexType::PumpFun,
            TradeTokenType::SOL,
            Pubkey::new_unique(),
            BuyAmount::ExactInput(10_000),
            dummy_pumpfun_params(),
            Hash::new_unique(),
            GasFeeStrategy::new(),
        )
        .durable_nonce(durable_nonce.clone());

        let low: TradeBuyParams = simple.into();

        assert!(low.recent_blockhash.is_none());
        assert_eq!(low.durable_nonce.as_ref().and_then(|n| n.nonce_account), Some(nonce_account));
        assert_eq!(low.durable_nonce.as_ref().and_then(|n| n.current_nonce), Some(nonce_hash));
    }

    #[test]
    fn simple_sell_auto_creates_non_sol_output_ata() {
        let simple = SimpleSellParams {
            dex_type: DexType::PumpFun,
            receive_as: TradeTokenType::USDC,
            mint: Pubkey::new_unique(),
            amount: SellAmount::ExactInput(50_000),
            slippage_basis_points: None,
            recent_blockhash: Some(Hash::new_unique()),
            extension_params: dummy_pumpfun_params(),
            gas_fee_strategy: GasFeeStrategy::new(),
            account_policy: AccountPolicy::Auto,
            address_lookup_table_accounts: Vec::new(),
            wait_tx_confirmed: false,
            wait_for_all_submits: false,
            durable_nonce: None,
            simulate: false,
            with_tip: true,
            grpc_recv_us: None,
        };

        let low: TradeSellParams = simple.into();

        assert!(matches!(low.output_token_type, TradeTokenType::USDC));
        assert_eq!(low.input_token_amount, 50_000);
        assert!(low.create_output_token_ata);
        assert!(!low.close_output_token_ata);
        assert!(!low.close_mint_token_ata);
    }

    #[test]
    fn simple_sell_builder_can_use_durable_nonce() {
        let nonce_account = Pubkey::new_unique();
        let nonce_hash = Hash::new_unique();
        let durable_nonce = DurableNonceInfo {
            nonce_account: Some(nonce_account),
            current_nonce: Some(nonce_hash),
        };

        let simple = SimpleSellParams::new(
            DexType::PumpFun,
            TradeTokenType::SOL,
            Pubkey::new_unique(),
            SellAmount::ExactInput(50_000),
            dummy_pumpfun_params(),
            Hash::new_unique(),
            GasFeeStrategy::new(),
        )
        .durable_nonce(durable_nonce.clone());

        let low: TradeSellParams = simple.into();

        assert!(low.recent_blockhash.is_none());
        assert_eq!(low.durable_nonce.as_ref().and_then(|n| n.nonce_account), Some(nonce_account));
        assert_eq!(low.durable_nonce.as_ref().and_then(|n| n.current_nonce), Some(nonce_hash));
    }
}
