use crate::swqos::SwqosConfig;
use solana_commitment_config::CommitmentConfig;
use std::hash::{Hash, Hasher};

/// Infrastructure-only configuration (wallet-independent)
/// Can be shared across multiple wallets using the same RPC/SWQOS setup
#[derive(Debug, Clone)]
pub struct InfrastructureConfig {
    pub rpc_url: String,
    pub swqos_configs: Vec<SwqosConfig>,
    pub commitment: CommitmentConfig,
}

impl InfrastructureConfig {
    pub fn new(
        rpc_url: String,
        swqos_configs: Vec<SwqosConfig>,
        commitment: CommitmentConfig,
    ) -> Self {
        Self { rpc_url, swqos_configs, commitment }
    }

    /// Create from TradeConfig (extract infrastructure-only settings)
    pub fn from_trade_config(config: &TradeConfig) -> Self {
        Self {
            rpc_url: config.rpc_url.clone(),
            swqos_configs: config.swqos_configs.clone(),
            commitment: config.commitment.clone(),
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
        // Hash commitment level as string since CommitmentConfig doesn't impl Hash
        format!("{:?}", self.commitment).hash(state);
    }
}

impl PartialEq for InfrastructureConfig {
    fn eq(&self, other: &Self) -> bool {
        self.rpc_url == other.rpc_url
            && self.swqos_configs == other.swqos_configs
            && self.commitment == other.commitment
    }
}

impl Eq for InfrastructureConfig {}

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
    /// Whether to pin parallel submit tasks to CPU cores (can reduce latency; set false in containers). Default true.
    pub use_core_affinity: bool,
    /// Whether to output all SDK logs (timing, SWQOS submit/confirm, WSOL, blacklist, etc.). Default true.
    pub log_enabled: bool,
    /// Whether to check minimum tip per SWQOS provider (filter out configs below min). Default false to save latency.
    pub check_min_tip: bool,
}

impl TradeConfig {
    pub fn new(
        rpc_url: String,
        swqos_configs: Vec<SwqosConfig>,
        commitment: CommitmentConfig,
    ) -> Self {
        if crate::common::sdk_log::sdk_log_enabled() {
            println!("🔧 TradeConfig create_wsol_ata_on_startup default: true");
            println!("🔧 TradeConfig use_seed_optimize default: true");
        }
        Self {
            rpc_url,
            swqos_configs,
            commitment,
            create_wsol_ata_on_startup: true, // default: check and create on startup
            use_seed_optimize: true,          // default: use seed optimization
            use_core_affinity: true,          // default: pin parallel submit tasks to cores
            log_enabled: true,                // default: enable all SDK logs
            check_min_tip: false,             // default: skip min tip check to reduce latency
        }
    }

    /// Create a TradeConfig with custom WSOL ATA settings
    pub fn with_wsol_ata_config(
        mut self,
        create_wsol_ata_on_startup: bool,
        use_seed_optimize: bool,
    ) -> Self {
        self.create_wsol_ata_on_startup = create_wsol_ata_on_startup;
        self.use_seed_optimize = use_seed_optimize;
        self
    }

    /// Set whether to check minimum tip per SWQOS (filter out configs below min). Default false for lower latency.
    pub fn with_check_min_tip(mut self, check_min_tip: bool) -> Self {
        self.check_min_tip = check_min_tip;
        self
    }
}

pub type SolanaRpcClient = solana_client::nonblocking::rpc_client::RpcClient;
pub type AnyResult<T> = anyhow::Result<T>;
