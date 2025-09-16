use crate::swqos::{SwqosClient, SwqosConfig};
use solana_sdk::commitment_config::CommitmentConfig;
use std::sync::Arc;

pub struct SwqosSettings {
    pub swqos_config: SwqosConfig,
    pub swqos_client: Option<Arc<SwqosClient>>,
    pub unit_limit: u32,
    pub unit_price: u64,
    pub buy_tip_fee: f64,
    pub sell_tip_fee: f64,
}

impl SwqosSettings {
    /// Create a new SwqosSettings
    /// swqos_config: SwqosConfig,
    /// unit_limit: u32,
    /// unit_price: u64,
    /// buy_tip_fee: f64,
    /// sell_tip_fee: f64,
    pub fn new(
        swqos_config: SwqosConfig,
        unit_limit: u32,
        unit_price: u64,
        buy_tip_fee: f64,
        sell_tip_fee: f64,
    ) -> Self {
        Self { swqos_config, swqos_client: None, unit_limit, unit_price, buy_tip_fee, sell_tip_fee }
    }

    pub fn setup_swqos_client(&mut self, rpc_url: String, commitment: CommitmentConfig) {
        let swqos_client =
            SwqosConfig::get_swqos_client(rpc_url, commitment, self.swqos_config.clone());
        self.swqos_client = Some(swqos_client);
    }

    pub fn clone(&self) -> Self {
        Self {
            swqos_config: self.swqos_config.clone(),
            swqos_client: self.swqos_client.clone(),
            unit_limit: self.unit_limit,
            unit_price: self.unit_price,
            buy_tip_fee: self.buy_tip_fee,
            sell_tip_fee: self.sell_tip_fee,
        }
    }
}
