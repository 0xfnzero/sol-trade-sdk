use crate::swqos::SwqosConfig;
use solana_sdk::commitment_config::CommitmentConfig;

#[derive(Debug, Clone)]
pub struct TradeConfig {
    pub rpc_url: String,
    pub swqos_configs: Vec<SwqosConfig>,
    pub commitment: CommitmentConfig,
}

impl TradeConfig {
    pub fn new(
        rpc_url: String,
        swqos_configs: Vec<SwqosConfig>,
        commitment: CommitmentConfig,
    ) -> Self {
        Self { rpc_url, swqos_configs, commitment }
    }
}

pub type SolanaRpcClient = solana_client::nonblocking::rpc_client::RpcClient;
pub type AnyResult<T> = anyhow::Result<T>;
