use std::sync::Arc;

use crate::{
    constants::trade::trade::{
        DEFAULT_BUY_TIP_FEE, DEFAULT_RPC_UNIT_LIMIT, DEFAULT_RPC_UNIT_PRICE, DEFAULT_SELL_TIP_FEE,
        DEFAULT_TIP_UNIT_LIMIT, DEFAULT_TIP_UNIT_PRICE,
    },
    swqos::{SwqosClient, SwqosConfig},
};
use serde::Deserialize;
use solana_client::rpc_client::RpcClient;
use solana_sdk::{commitment_config::CommitmentConfig, signature::Keypair};

#[derive(Debug, Clone)]
pub struct TradeConfig {
    pub rpc_url: String,
    pub swqos_configs: Vec<SwqosConfig>,
    pub priority_fee: PriorityFee,
    pub commitment: CommitmentConfig,
}

impl TradeConfig {
    pub fn new(
        rpc_url: String,
        swqos_configs: Vec<SwqosConfig>,
        priority_fee: PriorityFee,
        commitment: CommitmentConfig,
    ) -> Self {
        Self { rpc_url, swqos_configs, priority_fee, commitment }
    }
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct PriorityFee {
    pub tip_unit_limit: u32,
    pub tip_unit_price: u64,
    pub rpc_unit_limit: u32,
    pub rpc_unit_price: u64,
    // Matches the order of swqos
    pub buy_tip_fees: Vec<f64>,
    // Matches the order of swqos
    pub sell_tip_fees: Vec<f64>,
}

impl Default for PriorityFee {
    fn default() -> Self {
        Self {
            tip_unit_limit: DEFAULT_TIP_UNIT_LIMIT,
            tip_unit_price: DEFAULT_TIP_UNIT_PRICE,
            rpc_unit_limit: DEFAULT_RPC_UNIT_LIMIT,
            rpc_unit_price: DEFAULT_RPC_UNIT_PRICE,
            // Matches the order of swqos
            buy_tip_fees: vec![DEFAULT_BUY_TIP_FEE],
            // Matches the order of swqos
            sell_tip_fees: vec![DEFAULT_SELL_TIP_FEE],
        }
    }
}

pub type SolanaRpcClient = solana_client::nonblocking::rpc_client::RpcClient;

pub struct MethodArgs {
    pub payer: Arc<Keypair>,
    pub rpc: Arc<RpcClient>,
    pub nonblocking_rpc: Arc<SolanaRpcClient>,
    pub jito_client: Arc<SwqosClient>,
}

impl MethodArgs {
    pub fn new(
        payer: Arc<Keypair>,
        rpc: Arc<RpcClient>,
        nonblocking_rpc: Arc<SolanaRpcClient>,
        jito_client: Arc<SwqosClient>,
    ) -> Self {
        Self { payer, rpc, nonblocking_rpc, jito_client }
    }
}

pub type AnyResult<T> = anyhow::Result<T>;
