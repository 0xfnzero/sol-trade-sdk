use crate::swqos::SwqosConfig;
use solana_commitment_config::CommitmentConfig;

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
}

impl TradeConfig {
    pub fn new(
        rpc_url: String,
        swqos_configs: Vec<SwqosConfig>,
        commitment: CommitmentConfig,
    ) -> Self {
        println!("🔧 TradeConfig create_wsol_ata_on_startup default value: true");
        println!("🔧 TradeConfig use_seed_optimize default value: true");
        Self {
            rpc_url,
            swqos_configs,
            commitment,
            create_wsol_ata_on_startup: true,  // 默认：启动时检查并创建
            use_seed_optimize: true,           // 默认：使用seed优化
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
}

pub type SolanaRpcClient = solana_client::nonblocking::rpc_client::RpcClient;
pub type AnyResult<T> = anyhow::Result<T>;
