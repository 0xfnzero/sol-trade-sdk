//! Production typed configuration for sol-trade-sdk.
//!
//! # Design
//! Every config group is a standalone struct that can be deserialized from its
//! own TOML section. Environment-variable interpolation is supported via `${VAR_NAME}`
//! syntax in string fields. Startup validation is called once via [`AppConfig::validate()`].
//!
//! # Usage
//! ```rust,ignore
//! let cfg = AppConfig::from_path("/etc/solbot/config.toml")?;
//! cfg.validate()?;
//! let trade_cfg: TradeConfig = cfg.into_trade_config();
//! ```

use serde::{Deserialize, Serialize};
use std::{path::Path, time::Duration};

// ---------------------------------------------------------------------------
// Top-level config
// ---------------------------------------------------------------------------

/// Production configuration — all 13 groups.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub network: NetworkConfig,
    pub rpc: RpcConfig,
    pub shredstream: ShredStreamConfig,
    pub protocols: ProtocolsConfig,
    pub submission: SubmissionConfig,
    pub wallet: WalletConfig,
    pub blockhash: BlockhashConfig,
    pub fees: FeeConfig,
    pub risk: RiskConfig,
    pub strategy: StrategyConfig,
    pub observability: ObservabilityConfig,
    pub storage: StorageConfig,
    pub runtime: RuntimeConfig,
    pub canary: CanaryConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            network: NetworkConfig::default(),
            rpc: RpcConfig::default(),
            shredstream: ShredStreamConfig::default(),
            protocols: ProtocolsConfig::default(),
            submission: SubmissionConfig::default(),
            wallet: WalletConfig::default(),
            blockhash: BlockhashConfig::default(),
            fees: FeeConfig::default(),
            risk: RiskConfig::default(),
            strategy: StrategyConfig::default(),
            observability: ObservabilityConfig::default(),
            storage: StorageConfig::default(),
            runtime: RuntimeConfig::default(),
            canary: CanaryConfig::default(),
        }
    }
}

impl AppConfig {
    /// Load from a TOML file, interpolating `${ENV_VAR}` placeholders.
    pub fn from_path<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path.as_ref()).map_err(|e| {
            anyhow::anyhow!("Cannot read config `{}`: {}", path.as_ref().display(), e)
        })?;
        Self::from_toml(&raw)
    }

    /// Parse from a TOML string with env-var interpolation.
    pub fn from_toml(raw: &str) -> anyhow::Result<Self> {
        let interpolated = interpolate_env_vars(raw);
        let config: AppConfig = toml::de::from_str(&interpolated)
            .map_err(|e| anyhow::anyhow!("Config parse error: {}", e))?;
        Ok(config)
    }

    /// Validate all config groups at startup. Returns `Ok(())` or first failure.
    pub fn validate(&self) -> anyhow::Result<()> {
        self.network.validate()?;
        self.rpc.validate()?;
        self.shredstream.validate()?;
        self.protocols.validate()?;
        self.submission.validate()?;
        self.wallet.validate()?;
        self.blockhash.validate()?;
        self.fees.validate()?;
        self.risk.validate()?;
        self.strategy.validate()?;
        self.observability.validate()?;
        self.storage.validate()?;
        self.runtime.validate()?;
        self.canary.validate()?;
        Ok(())
    }

    /// Convert to a `crate::common::TradeConfig` for backward compatibility.
    pub fn into_trade_config(&self) -> crate::common::TradeConfig {
        use crate::common::TradeConfig;

        let swqos_configs = self.submission.to_swqos_configs();
        let commitment = self.rpc.commitment_config();

        TradeConfig::builder(self.rpc.primary.clone(), swqos_configs, commitment).build()
    }
}

// ---------------------------------------------------------------------------
// 1. Network
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    pub rpc_url: String,
    pub rpc_fallback_url: String,
    pub ws_url: String,
    pub connection_timeout_secs: u64,
    pub request_timeout_secs: u64,
    pub rate_limit_rps: u32,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            rpc_url: "https://api.mainnet-beta.solana.com".into(),
            rpc_fallback_url: "https://solana-api.projectserum.com".into(),
            ws_url: "wss://api.mainnet-beta.solana.com".into(),
            connection_timeout_secs: 10,
            request_timeout_secs: 30,
            rate_limit_rps: 100,
        }
    }
}

impl NetworkConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.connection_timeout_secs == 0 {
            anyhow::bail!("network.connection_timeout_secs must be > 0");
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 2. RPC
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RpcConfig {
    pub primary: String,
    pub fallback: String,
    pub commitment: String,
    pub rate_limit_per_second: u32,
    pub max_retries: u32,
    pub retry_backoff_ms: Vec<u64>,
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            primary: "https://api.mainnet-beta.solana.com".into(),
            fallback: "https://solana-api.projectserum.com".into(),
            commitment: "processed".into(),
            rate_limit_per_second: 100,
            max_retries: 3,
            retry_backoff_ms: vec![100, 500, 2000],
        }
    }
}

impl RpcConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        match self.commitment.as_str() {
            "processed" | "confirmed" | "finalized" => {}
            other => anyhow::bail!(
                "rpc.commitment must be processed/confirmed/finalized, got `{}`",
                other
            ),
        }
        Ok(())
    }

    pub fn commitment_config(&self) -> solana_commitment_config::CommitmentConfig {
        match self.commitment.as_str() {
            "confirmed" => solana_commitment_config::CommitmentConfig::confirmed(),
            "finalized" => solana_commitment_config::CommitmentConfig::finalized(),
            _ => solana_commitment_config::CommitmentConfig::processed(),
        }
    }
}

// ---------------------------------------------------------------------------
// 3. ShredStream
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ShredStreamConfig {
    pub enabled: bool,
    pub udp_bind_addr: String,
    pub receive_buffer_bytes: u32,
    pub busy_poll_usec: u32,
    pub slot_window: u32,
    pub fec_enabled: bool,
    pub stuck_batch_timeout_ms: u64,
    pub program_ids: Vec<String>,
    pub cpu_affinity: CpuAffinityConfig,
}

impl Default for ShredStreamConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            udp_bind_addr: "0.0.0.0:8001".into(),
            receive_buffer_bytes: 67_108_864, // 64 MiB
            busy_poll_usec: 200,
            slot_window: 3,
            fec_enabled: true,
            stuck_batch_timeout_ms: 50,
            program_ids: vec![
                "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P".into(), // PumpFun
                "pAMMBay6oceH9fJKBRHGP5D4bD8sKbwv3Y5SQE2sTjQ".into(), // PumpSwap
                "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8".into(), // Raydium AMM V4
                "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C".into(), // Raydium CPMM
                "LBUZKhRxPF3XUpBCjp4YzTKgY6bE1UEsLzD2jF5K5oM".into(), // Meteora
                "po9o1HhL9G9BKkBCiJN6KSm4hK7j4G4G4K4G4K4G4K4G".into(), // Bonk
            ],
            cpu_affinity: CpuAffinityConfig::default(),
        }
    }
}

impl ShredStreamConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.enabled && self.receive_buffer_bytes < 1_048_576 {
            anyhow::bail!("shredstream.receive_buffer_bytes must be >= 1 MiB when enabled");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CpuAffinityConfig {
    pub receive_core: u32,
    pub reconstruct_core: u32,
    pub state_strategy_core: u32,
    pub execute_core: u32,
}

impl Default for CpuAffinityConfig {
    fn default() -> Self {
        Self { receive_core: 0, reconstruct_core: 1, state_strategy_core: 2, execute_core: 3 }
    }
}

// ---------------------------------------------------------------------------
// 4. Protocols
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProtocolsConfig {
    pub enabled: Vec<String>,
    pub disabled: Vec<String>,
    pub pumpfun: ProtocolEntry,
    pub pumpswap: ProtocolEntry,
    pub raydium_amm_v4: ProtocolEntry,
    pub raydium_cpmm: ProtocolEntry,
    pub meteora_damm_v2: ProtocolEntry,
    pub bonk: ProtocolEntry,
}

impl Default for ProtocolsConfig {
    fn default() -> Self {
        Self {
            enabled: vec!["pumpfun".into(), "pumpswap".into(), "raydium_cpmm".into()],
            disabled: vec![],
            pumpfun: ProtocolEntry { max_quote_age_ms: 500, min_liquidity_sol: 5.0 },
            pumpswap: ProtocolEntry { max_quote_age_ms: 300, min_liquidity_sol: 1.0 },
            raydium_amm_v4: ProtocolEntry { max_quote_age_ms: 200, min_liquidity_sol: 1.0 },
            raydium_cpmm: ProtocolEntry { max_quote_age_ms: 200, min_liquidity_sol: 1.0 },
            meteora_damm_v2: ProtocolEntry { max_quote_age_ms: 200, min_liquidity_sol: 1.0 },
            bonk: ProtocolEntry { max_quote_age_ms: 500, min_liquidity_sol: 0.5 },
        }
    }
}

impl ProtocolsConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        // Check no protocol is both enabled and disabled
        for e in &self.enabled {
            if self.disabled.contains(e) {
                anyhow::bail!("protocol `{}` is listed in both enabled and disabled", e);
            }
        }
        Ok(())
    }

    pub fn is_protocol_enabled(&self, name: &str) -> bool {
        self.enabled.contains(&name.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProtocolEntry {
    pub max_quote_age_ms: u64,
    pub min_liquidity_sol: f64,
}

impl Default for ProtocolEntry {
    fn default() -> Self {
        Self { max_quote_age_ms: 300, min_liquidity_sol: 1.0 }
    }
}

// ---------------------------------------------------------------------------
// 5. Submission
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SubmissionConfig {
    pub primary_provider: String,
    pub fallback_provider: String,
    pub max_simultaneous_lanes: u32,
    pub require_same_signature: bool,
    pub jito: JitoSubmissionConfig,
}

impl Default for SubmissionConfig {
    fn default() -> Self {
        Self {
            primary_provider: "rpc".into(),
            fallback_provider: "jito".into(),
            max_simultaneous_lanes: 2,
            require_same_signature: true,
            jito: JitoSubmissionConfig::default(),
        }
    }
}

impl SubmissionConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.max_simultaneous_lanes == 0 || self.max_simultaneous_lanes > 8 {
            anyhow::bail!("submission.max_simultaneous_lanes must be 1-8");
        }
        Ok(())
    }

    /// Convert to existing SWQoS config vec (bridge to existing SDK types).
    pub fn to_swqos_configs(&self) -> Vec<crate::swqos::SwqosConfig> {
        let mut configs = Vec::with_capacity(self.max_simultaneous_lanes as usize);

        // Primary RPC lane
        configs
            .push(crate::swqos::SwqosConfig::Default("https://api.mainnet-beta.solana.com".into()));

        // Jito lane if enabled
        if self.jito.endpoint != "disabled" {
            configs.push(crate::swqos::SwqosConfig::Jito(
                self.jito.auth_token.clone(),
                crate::swqos::SwqosRegion::Default,
                Some(self.jito.endpoint.clone()),
            ));
        }

        configs.truncate(self.max_simultaneous_lanes as usize);
        configs
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct JitoSubmissionConfig {
    pub endpoint: String,
    pub auth_token: String,
    pub tip_min_lamports: u64,
    pub tip_max_lamports: u64,
    pub bundle_tip_strategy: String,
}

impl Default for JitoSubmissionConfig {
    fn default() -> Self {
        Self {
            endpoint: "https://mainnet.block-engine.jito.wtf".into(),
            auth_token: String::new(),
            tip_min_lamports: 1_000,
            tip_max_lamports: 100_000,
            bundle_tip_strategy: "fixed".into(),
        }
    }
}

// ---------------------------------------------------------------------------
// 6. Wallet
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WalletConfig {
    pub execution_keypair_path: String,
    pub payer_keypair_path: String,
    pub min_balance_sol: f64,
    pub max_balance_sol: f64,
}

impl Default for WalletConfig {
    fn default() -> Self {
        Self {
            execution_keypair_path: "/etc/solbot/execution-keypair.json".into(),
            payer_keypair_path: "/etc/solbot/payer-keypair.json".into(),
            min_balance_sol: 0.5,
            max_balance_sol: 10.0,
        }
    }
}

impl WalletConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.min_balance_sol <= 0.0 {
            anyhow::bail!("wallet.min_balance_sol must be > 0");
        }
        if self.max_balance_sol <= self.min_balance_sol {
            anyhow::bail!("wallet.max_balance_sol must be > min_balance_sol");
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 7. Blockhash
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BlockhashConfig {
    pub refresh_interval_ms: u64,
    pub max_age_slots: u64,
    pub stale_threshold_ms: u64,
    pub commitment: String,
    pub fail_on_lag: bool,
}

impl Default for BlockhashConfig {
    fn default() -> Self {
        Self {
            refresh_interval_ms: 200,
            max_age_slots: 150,
            stale_threshold_ms: 500,
            commitment: "confirmed".into(),
            fail_on_lag: true,
        }
    }
}

impl BlockhashConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.refresh_interval_ms < 50 {
            anyhow::bail!("blockhash.refresh_interval_ms must be >= 50ms");
        }
        Ok(())
    }

    pub fn refresh_interval(&self) -> Duration {
        Duration::from_millis(self.refresh_interval_ms)
    }
}

// ---------------------------------------------------------------------------
// 8. Fees
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FeeConfig {
    pub compute_unit_limit: u32,
    pub compute_unit_price: u64,
    pub max_priority_fee_lamports: u64,
    pub max_relay_tip_lamports: u64,
    pub max_total_tx_cost_lamports: u64,
    pub profit_threshold_lamports: u64,
    pub cu_tracking_enabled: bool,
    pub cu_tracking_window: u32,
}

impl Default for FeeConfig {
    fn default() -> Self {
        Self {
            compute_unit_limit: 400_000,
            compute_unit_price: 1_000,
            max_priority_fee_lamports: 10_000,
            max_relay_tip_lamports: 5_000,
            max_total_tx_cost_lamports: 20_000,
            profit_threshold_lamports: 1_000,
            cu_tracking_enabled: true,
            cu_tracking_window: 100,
        }
    }
}

impl FeeConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.compute_unit_limit < 100_000 {
            anyhow::bail!("fees.compute_unit_limit must be >= 100_000");
        }
        if self.cu_tracking_window < 10 {
            anyhow::bail!("fees.cu_tracking_window must be >= 10");
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 9. Risk
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RiskConfig {
    // Per-trade (8)
    pub max_sol_per_trade: f64,
    pub max_token_amount_per_trade: u64,
    pub max_slippage_basis_points: u64,
    pub max_quote_age_ms: u64,
    pub max_source_slot_age: u64,
    pub min_expected_output: u64,
    pub min_expected_output_as_bps: bool,
    pub min_expected_net_profit_lamports: u64,

    // Global (8)
    pub global: GlobalRiskConfig,

    // Fee ceilings (7)
    pub fee_ceilings: FeeCeilingConfig,

    // Health (6)
    pub health: HealthRiskConfig,

    // Switches (6)
    pub switches: SwitchConfig,
}

impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            max_sol_per_trade: 0.1,
            max_token_amount_per_trade: 500_000_000,
            max_slippage_basis_points: 100,
            max_quote_age_ms: 200,
            max_source_slot_age: 2,
            min_expected_output: 1,
            min_expected_output_as_bps: false,
            min_expected_net_profit_lamports: 500,
            global: GlobalRiskConfig::default(),
            fee_ceilings: FeeCeilingConfig::default(),
            health: HealthRiskConfig::default(),
            switches: SwitchConfig::default(),
        }
    }
}

impl RiskConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.max_slippage_basis_points > 10_000 {
            anyhow::bail!("risk.max_slippage_basis_points must be <= 10000 (100%)");
        }
        self.global.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GlobalRiskConfig {
    pub max_open_exposure_sol: f64,
    pub max_exposure_per_mint_pct: f64,
    pub max_exposure_per_protocol_pct: f64,
    pub max_tx_per_minute: u32,
    pub max_failures_per_5min: u32,
    pub per_mint_cap_sol: f64,
    pub max_positions_per_protocol: u32,
    pub max_daily_trades: u32,
    pub max_open_positions: u32,
}

impl Default for GlobalRiskConfig {
    fn default() -> Self {
        Self {
            max_open_exposure_sol: 1.0,
            max_exposure_per_mint_pct: 25.0,
            max_exposure_per_protocol_pct: 50.0,
            max_tx_per_minute: 60,
            max_failures_per_5min: 10,
            per_mint_cap_sol: 0.5,
            max_positions_per_protocol: 3,
            max_daily_trades: 500,
            max_open_positions: 5,
        }
    }
}

impl GlobalRiskConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.max_exposure_per_mint_pct > 100.0 {
            anyhow::bail!("risk.global.max_exposure_per_mint_pct must be <= 100%");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FeeCeilingConfig {
    pub max_priority_fee_lamports: u64,
    pub max_relay_tip_lamports: u64,
    pub max_total_tx_cost_lamports: u64,
    pub max_daily_loss_lamports: u64,
    pub max_consecutive_losses: u32,
    pub max_net_loss_per_trade_lamports: u64,
    pub max_loss_rate_per_100_trades_pct: f64,
}

impl Default for FeeCeilingConfig {
    fn default() -> Self {
        Self {
            max_priority_fee_lamports: 10_000,
            max_relay_tip_lamports: 5_000,
            max_total_tx_cost_lamports: 20_000,
            max_daily_loss_lamports: 500_000,
            max_consecutive_losses: 3,
            max_net_loss_per_trade_lamports: 50_000,
            max_loss_rate_per_100_trades_pct: 60.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HealthRiskConfig {
    pub max_rpc_slot_lag: u64,
    pub max_packet_loss_pct: f64,
    pub max_queue_delay_ms: u64,
    pub max_blockhash_age_ms: u64,
    pub max_reconciliation_backlog: u32,
    pub max_shred_loss_pct: f64,
}

impl Default for HealthRiskConfig {
    fn default() -> Self {
        Self {
            max_rpc_slot_lag: 5,
            max_packet_loss_pct: 1.0,
            max_queue_delay_ms: 50,
            max_blockhash_age_ms: 500,
            max_reconciliation_backlog: 100,
            max_shred_loss_pct: 2.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SwitchConfig {
    pub global_kill: bool,
    pub buy_only: bool,
    pub sell_only: bool,
    pub per_protocol_disabled: Vec<String>,
    pub per_provider_disabled: Vec<String>,
    pub per_token_denylist: Vec<String>,
}

impl Default for SwitchConfig {
    fn default() -> Self {
        Self {
            global_kill: false,
            buy_only: false,
            sell_only: false,
            per_protocol_disabled: vec![],
            per_provider_disabled: vec![],
            per_token_denylist: vec![],
        }
    }
}

impl SwitchConfig {
    pub fn is_allowed(&self, protocol: &str, provider: &str, mint: &str) -> bool {
        if self.global_kill {
            return false;
        }
        if self.per_protocol_disabled.contains(&protocol.to_string()) {
            return false;
        }
        if self.per_provider_disabled.contains(&provider.to_string()) {
            return false;
        }
        if self.per_token_denylist.contains(&mint.to_string()) {
            return false;
        }
        true
    }
}

// ---------------------------------------------------------------------------
// 10. Strategy
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StrategyConfig {
    pub strategy_type: String,
    pub max_open_positions: u32,
    pub position_size_sol: f64,
    pub take_profit_basis_points: u64,
    pub stop_loss_basis_points: u64,
    pub max_hold_ms: u64,
    pub min_slot_distance: u64,
}

impl Default for StrategyConfig {
    fn default() -> Self {
        Self {
            strategy_type: "arbitrage".into(),
            max_open_positions: 1,
            position_size_sol: 0.05,
            take_profit_basis_points: 50,
            stop_loss_basis_points: 30,
            max_hold_ms: 30_000,
            min_slot_distance: 1,
        }
    }
}

impl StrategyConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.max_open_positions == 0 {
            anyhow::bail!("strategy.max_open_positions must be >= 1");
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 11. Observability
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ObservabilityConfig {
    pub metrics_endpoint: String,
    pub metrics_prefix: String,
    pub log_level: String,
    pub log_format: String,
    pub log_sampling_rate: f64,
    pub trace_storage_path: String,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            metrics_endpoint: "0.0.0.0:9090".into(),
            metrics_prefix: "solbot".into(),
            log_level: "info".into(),
            log_format: "json".into(),
            log_sampling_rate: 0.1,
            trace_storage_path: "/var/log/solbot/traces".into(),
        }
    }
}

impl ObservabilityConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        match self.log_level.as_str() {
            "error" | "warn" | "info" | "debug" | "trace" => {}
            other => anyhow::bail!(
                "observability.log_level must be error/warn/info/debug/trace, got `{}`",
                other
            ),
        }
        match self.log_format.as_str() {
            "json" | "plain" => {}
            other => anyhow::bail!("observability.log_format must be json/plain, got `{}`", other),
        }
        if !(0.0..=1.0).contains(&self.log_sampling_rate) {
            anyhow::bail!("observability.log_sampling_rate must be 0.0..=1.0");
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 12. Storage
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    pub state_path: String,
    pub dedup_cache_size: u32,
    pub reconciliation_db: String,
    pub max_disk_usage_gb: f64,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            state_path: "/var/lib/solbot/state".into(),
            dedup_cache_size: 262_144,
            reconciliation_db: "/var/lib/solbot/reconciliation.db".into(),
            max_disk_usage_gb: 10.0,
        }
    }
}

impl StorageConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.dedup_cache_size < 1024 {
            anyhow::bail!("storage.dedup_cache_size must be >= 1024");
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 13. Runtime
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeConfig {
    pub graceful_shutdown_timeout_secs: u64,
    pub restart_max_retries: u32,
    pub restart_delay_ms: u64,
    pub cpu_affinity_enabled: bool,
    pub memory_limit_mb: u64,
    pub file_descriptor_limit: u64,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            graceful_shutdown_timeout_secs: 10,
            restart_max_retries: 3,
            restart_delay_ms: 1_000,
            cpu_affinity_enabled: true,
            memory_limit_mb: 1024,
            file_descriptor_limit: 65536,
        }
    }
}

impl RuntimeConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.graceful_shutdown_timeout_secs < 1 {
            anyhow::bail!("runtime.graceful_shutdown_timeout_secs must be >= 1");
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 14. Canary
// ---------------------------------------------------------------------------

/// Canary mode configuration — tiny wallet, tight limits, manual triggers.
///
/// Phase 12 of the productionization roadmap: run alongside shadow mode
/// with a tiny wallet, ultra-conservative limits, and manual approval
/// gate to compare hypothetical vs real outcomes before going full prod.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CanaryConfig {
    /// Enable canary mode. When enabled, the solcanary binary enforces
    /// ultra-tight limits and optionally requires manual approval.
    pub enabled: bool,

    /// Maximum SOL balance allowed in the wallet at startup.
    /// solcanary will refuse to start if balance exceeds this.
    pub max_wallet_balance_sol: f64,

    /// Override for max SOL per trade (even tighter than risk defaults).
    pub max_sol_per_trade: f64,

    /// Override for max daily trades (even tighter than risk defaults).
    pub max_daily_trades: u32,

    /// Override for max open positions.
    pub max_open_positions: u32,

    /// Require manual HTTP approval before each trade.
    pub manual_approval: bool,

    /// How long to wait for manual approval before auto-rejecting (seconds).
    pub approval_timeout_secs: u64,

    /// Enable shadow comparison — records shadow decisions alongside
    /// actual trades for outcome comparison.
    pub shadow_comparison: bool,

    /// Path to the shadow decisions DB file (SQLite), for cross-referencing.
    pub shadow_db_path: String,

    /// Allowlist of protocols that canary mode will trade.
    /// Empty = all enabled protocols allowed.
    pub allowed_protocols: Vec<String>,

    /// Max slippage basis points for canary trades (even tighter).
    pub max_slippage_basis_points: u64,

    /// Require at least N SOL balance before starting canary (anti-drain).
    pub min_wallet_balance_sol: f64,
}

impl Default for CanaryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_wallet_balance_sol: 0.5,
            max_sol_per_trade: 0.01,
            max_daily_trades: 20,
            max_open_positions: 1,
            manual_approval: true,
            approval_timeout_secs: 60,
            shadow_comparison: true,
            shadow_db_path: "/var/lib/solbot/shadow.db".into(),
            allowed_protocols: vec![],
            max_slippage_basis_points: 25,
            min_wallet_balance_sol: 0.05,
        }
    }
}

impl CanaryConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.enabled {
            if self.max_wallet_balance_sol <= 0.0 {
                anyhow::bail!("canary.max_wallet_balance_sol must be > 0 when enabled");
            }
            if self.max_wallet_balance_sol > 5.0 {
                anyhow::bail!("canary.max_wallet_balance_sol must be <= 5.0 (tiny wallet)");
            }
            if self.max_sol_per_trade > 0.1 {
                anyhow::bail!("canary.max_sol_per_trade must be <= 0.1 (tiny position)");
            }
            if self.max_daily_trades > 100 {
                anyhow::bail!("canary.max_daily_trades must be <= 100");
            }
            if self.approval_timeout_secs < 5 {
                anyhow::bail!("canary.approval_timeout_secs must be >= 5");
            }
            if self.min_wallet_balance_sol <= 0.0 {
                anyhow::bail!("canary.min_wallet_balance_sol must be > 0");
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Env-var interpolation
// ---------------------------------------------------------------------------

/// Replace `${VAR_NAME}` (or `${VAR_NAME:-default}`) placeholders with env vars.
fn interpolate_env_vars(raw: &str) -> String {
    let re = regex::Regex::new(r"\$\{([^:}-]+)(?::-([^}]*))?\}").unwrap();
    re.replace_all(raw, |caps: &regex::Captures| {
        let var = &caps[1];
        let default = caps.get(2).map(|m| m.as_str());
        std::env::var(var).unwrap_or_else(|_| default.unwrap_or("").to_string())
    })
    .into_owned()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_parses_and_validates() {
        let cfg = AppConfig::default();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn from_toml_minimal() {
        let toml_str = r#"
[network]
rpc_url = "https://api.testnet.solana.com"
        "#;
        let cfg = AppConfig::from_toml(toml_str).unwrap();
        assert_eq!(cfg.network.rpc_url, "https://api.testnet.solana.com");
        // Other fields should be default
        assert_eq!(cfg.network.connection_timeout_secs, 10);
    }

    #[test]
    fn env_var_interpolation() {
        // Test with a variable that's definitely unset — should use default
        let toml_str = r#"
[wallet]
execution_keypair_path = "${SOLBOT_HOME_NONEXISTENT:-/tmp/solbot}/id.json"
        "#;
        let cfg = AppConfig::from_toml(toml_str).unwrap();
        assert_eq!(cfg.wallet.execution_keypair_path, "/tmp/solbot/id.json");
    }

    #[test]
    fn validate_rejects_bad_commitment() {
        let toml_str = r#"
[rpc]
commitment = "pending"
        "#;
        let cfg = AppConfig::from_toml(toml_str).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn into_trade_config_preserves_rpc() {
        let cfg = AppConfig::default();
        let tc = cfg.into_trade_config();
        assert!(tc.rpc_url.contains("mainnet"));
    }

    #[test]
    fn switch_config_blocks_on_global_kill() {
        let switches = SwitchConfig { global_kill: true, ..Default::default() };
        assert!(!switches.is_allowed("pumpfun", "rpc", "mint123"));
    }

    #[test]
    fn switch_config_blocks_denied_protocol() {
        let switches =
            SwitchConfig { per_protocol_disabled: vec!["bonk".into()], ..Default::default() };
        assert!(switches.is_allowed("pumpfun", "rpc", "x"));
        assert!(!switches.is_allowed("bonk", "rpc", "x"));
    }
}
