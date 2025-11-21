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
    /// Whether to use seed optimization for WSOL ATA operations (default: false)
    pub wsol_use_seed: bool,
    /// Whether to use seed optimization for other mint ATA operations (default: true)
    pub mint_use_seed: bool,
}

impl TradeConfig {
    pub fn new(
        rpc_url: String,
        swqos_configs: Vec<SwqosConfig>,
        commitment: CommitmentConfig,
        create_wsol_ata_on_startup: bool,
        wsol_use_seed: bool,
        mint_use_seed: bool,
    ) -> Self {
        Self {
            rpc_url,
            swqos_configs,
            commitment,
            create_wsol_ata_on_startup,
            wsol_use_seed,
            mint_use_seed,
        }
    }
}

pub type SolanaRpcClient = solana_client::nonblocking::rpc_client::RpcClient;
pub type AnyResult<T> = anyhow::Result<T>;
