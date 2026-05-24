use crate::common::GasFeeStrategyType;
use crate::swqos::{SwqosConfig, SwqosType};
use solana_commitment_config::CommitmentConfig;
use std::hash::{Hash, Hasher};

/// Infrastructure-only configuration (wallet-independent)
/// Can be shared across multiple wallets using the same RPC/SWQOS setup
#[derive(Debug, Clone)]
pub struct InfrastructureConfig {
    pub rpc_url: String,
    pub swqos_configs: Vec<SwqosConfig>,
    pub commitment: CommitmentConfig,
    /// When true, SWQOS sender threads use the *last* N cores instead of the first N. Reduces contention with main thread / default tokio workers that often use low-numbered cores. Default false.
    pub swqos_cores_from_end: bool,
    /// Global MEV protection flag. When true, SWQOS providers that support MEV protection
    /// (Astralane QUIC `:9000` or HTTP `mev-protect=true`, BlockRazor) use MEV-protected
    /// endpoints/modes. Default false.
    pub mev_protection: bool,
}

impl InfrastructureConfig {
    pub fn new(
        rpc_url: String,
        swqos_configs: Vec<SwqosConfig>,
        commitment: CommitmentConfig,
    ) -> Self {
        Self {
            rpc_url,
            swqos_configs,
            commitment,
            swqos_cores_from_end: false,
            mev_protection: false,
        }
    }

    /// Create from TradeConfig (extract infrastructure-only settings)
    pub fn from_trade_config(config: &TradeConfig) -> Self {
        Self {
            rpc_url: config.rpc_url.clone(),
            swqos_configs: config.swqos_configs.clone(),
            commitment: config.commitment.clone(),
            swqos_cores_from_end: config.swqos_cores_from_end,
            mev_protection: config.mev_protection,
        }
    }

    /// Generate a cache key for this infrastructure configuration
    pub fn cache_key(&self) -> String {
        use std::collections::hash_map::DefaultHasher;
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }
}

// Manual Hash implementation since CommitmentConfig doesn't implement Hash
impl Hash for InfrastructureConfig {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.rpc_url.hash(state);
        self.swqos_configs.hash(state);
        format!("{:?}", self.commitment).hash(state);
        self.swqos_cores_from_end.hash(state);
        self.mev_protection.hash(state);
    }
}

impl PartialEq for InfrastructureConfig {
    fn eq(&self, other: &Self) -> bool {
        self.rpc_url == other.rpc_url
            && self.swqos_configs == other.swqos_configs
            && self.commitment == other.commitment
            && self.swqos_cores_from_end == other.swqos_cores_from_end
            && self.mev_protection == other.mev_protection
    }
}

impl Eq for InfrastructureConfig {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwqosSubmitTiming {
    pub swqos_type: SwqosType,
    pub strategy_type: GasFeeStrategyType,
    pub submit_done_us: i64,
}

#[derive(Debug, Clone)]
pub struct TradeConfig {
    pub rpc_url: String,
    pub swqos_configs: Vec<SwqosConfig>,
    pub commitment: CommitmentConfig,
    /// Whether to create WSOL ATA on startup (default: true)
    /// If true, SDK will check WSOL ATA on initialization and create if not exists
    pub create_wsol_ata_on_startup: bool,
    /// Whether to use seed optimization for all ATA operations (default: true)
    pub use_seed_optimize: bool,
    /// Whether to output all SDK logs (timing, SWQOS submit/confirm, WSOL, blacklist, etc.). Default true.
    pub log_enabled: bool,
    /// Whether to check minimum tip per SWQOS provider (filter out configs below min). Default false to save latency.
    pub check_min_tip: bool,
    /// When true, SWQOS uses the *last* N cores (instead of the first N). Use when main thread / tokio use low-numbered cores to reduce CPU contention. Default false.
    pub swqos_cores_from_end: bool,
    /// Global MEV protection flag. When true, SWQOS providers that support MEV protection
    /// (Astralane QUIC `:9000` or Plain/Binary HTTP `mev-protect=true`, BlockRazor sandwichMitigation)
    /// use their MEV-protected endpoints/modes. Default false (no MEV protection, lower latency).
    pub mev_protection: bool,
}

impl TradeConfig {
    /// Create a new TradeConfig using the builder pattern.
    ///
    /// # Available builder methods
    /// - `.create_wsol_ata_on_startup(bool)` — check & create WSOL ATA on init (default: true)
    /// - `.use_seed_optimize(bool)`           — seed optimization for ATA ops (default: true)
    /// - `.log_enabled(bool)`                 — SDK timing/SWQOS logs (default: true)
    /// - `.check_min_tip(bool)`               — filter SWQOS below min tip (default: false)
    /// - `.swqos_cores_from_end(bool)`        — bind SWQOS to last N cores (default: false)
    /// - `.mev_protection(bool)`              — MEV protection for Astralane/BlockRazor (default: false)
    ///
    /// # Example
    /// ```rust,ignore
    /// let config = TradeConfig::builder(rpc_url, swqos_configs, commitment)
    ///     .mev_protection(true)
    ///     .check_min_tip(true)
    ///     .log_enabled(false)
    ///     .build();
    /// ```
    pub fn builder(
        rpc_url: String,
        swqos_configs: Vec<SwqosConfig>,
        commitment: CommitmentConfig,
    ) -> TradeConfigBuilder {
        TradeConfigBuilder::new(rpc_url, swqos_configs, commitment)
    }

    /// Shortcut: create a TradeConfig with all defaults. Equivalent to `builder(...).build()`.
    pub fn new(
        rpc_url: String,
        swqos_configs: Vec<SwqosConfig>,
        commitment: CommitmentConfig,
    ) -> Self {
        Self::builder(rpc_url, swqos_configs, commitment).build()
    }
}

/// Builder for [`TradeConfig`]. Created via [`TradeConfig::builder`].
///
/// All fields are optional and pre-filled with sensible defaults.
/// Call `.build()` to produce the final [`TradeConfig`].
#[derive(Debug, Clone)]
pub struct TradeConfigBuilder {
    rpc_url: String,
    swqos_configs: Vec<SwqosConfig>,
    commitment: CommitmentConfig,
    create_wsol_ata_on_startup: bool,
    use_seed_optimize: bool,
    log_enabled: bool,
    check_min_tip: bool,
    swqos_cores_from_end: bool,
    mev_protection: bool,
}

impl TradeConfigBuilder {
    fn new(rpc_url: String, swqos_configs: Vec<SwqosConfig>, commitment: CommitmentConfig) -> Self {
        Self {
            rpc_url,
            swqos_configs,
            commitment,
            create_wsol_ata_on_startup: true,
            use_seed_optimize: true,
            log_enabled: true,
            check_min_tip: false,
            swqos_cores_from_end: false,
            mev_protection: false,
        }
    }

    /// Check and create WSOL ATA on SDK initialization. Default: `true`.
    pub fn create_wsol_ata_on_startup(mut self, v: bool) -> Self {
        self.create_wsol_ata_on_startup = v;
        self
    }

    /// Enable seed optimization for all ATA operations. Default: `true`.
    pub fn use_seed_optimize(mut self, v: bool) -> Self {
        self.use_seed_optimize = v;
        self
    }

    /// Enable SDK logs (timing, SWQOS submit/confirm, WSOL, blacklist, etc.). Default: `true`.
    pub fn log_enabled(mut self, v: bool) -> Self {
        self.log_enabled = v;
        self
    }

    /// Filter out SWQOS providers whose tip is below their minimum requirement.
    /// Adds a small check on the hot path; disable for lowest latency. Default: `false`.
    pub fn check_min_tip(mut self, v: bool) -> Self {
        self.check_min_tip = v;
        self
    }

    /// Bind SWQOS sender threads to the *last* N CPU cores instead of the first N.
    /// Useful when main thread / tokio workers occupy low-numbered cores. Default: `false`.
    pub fn swqos_cores_from_end(mut self, v: bool) -> Self {
        self.swqos_cores_from_end = v;
        self
    }

    /// Enable global MEV protection. When `true`:
    /// - **Astralane QUIC** uses port `9000`; **Astralane HTTP** adds `mev-protect=true`
    /// - **BlockRazor** uses `mode=sandwichMitigation` (skips blacklisted Leader slots)
    ///
    /// May reduce landing speed. Default: `false`.
    pub fn mev_protection(mut self, v: bool) -> Self {
        self.mev_protection = v;
        self
    }

    /// Consume the builder and produce a [`TradeConfig`].
    pub fn build(self) -> TradeConfig {
        TradeConfig {
            rpc_url: self.rpc_url,
            swqos_configs: self.swqos_configs,
            commitment: self.commitment,
            create_wsol_ata_on_startup: self.create_wsol_ata_on_startup,
            use_seed_optimize: self.use_seed_optimize,
            log_enabled: self.log_enabled,
            check_min_tip: self.check_min_tip,
            swqos_cores_from_end: self.swqos_cores_from_end,
            mev_protection: self.mev_protection,
        }
    }
}

pub type SolanaRpcClient = solana_client::nonblocking::rpc_client::RpcClient;
pub type AnyResult<T> = anyhow::Result<T>;
