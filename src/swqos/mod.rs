pub mod common;
pub mod solana_rpc;
pub mod jito;
pub mod nextblock;
pub mod zeroslot;
pub mod temporal;
pub mod bloxroute;

use std::sync::Arc;

use solana_sdk::{commitment_config::CommitmentConfig, transaction::VersionedTransaction};
use tokio::sync::RwLock;

use anyhow::Result;

use crate::{common::SolanaRpcClient, constants::swqos::{SWQOS_ENDPOINTS_BLOX, SWQOS_ENDPOINTS_JITO, SWQOS_ENDPOINTS_NEXTBLOCK, SWQOS_ENDPOINTS_TEMPORAL, SWQOS_ENDPOINTS_ZERO_SLOT}, swqos::{bloxroute::BloxrouteClient, jito::JitoClient, nextblock::NextBlockClient, solana_rpc::SolRpcClient, temporal::TemporalClient, zeroslot::ZeroSlotClient}};

lazy_static::lazy_static! {
    static ref TIP_ACCOUNT_CACHE: RwLock<Vec<String>> = RwLock::new(Vec::new());
}

#[derive(Debug, Clone, Copy)]
pub enum TradeType {
    Create,
    CreateAndBuy,
    Buy,
    Sell,
}

impl std::fmt::Display for TradeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            TradeType::Create => "Create",
            TradeType::CreateAndBuy => "Create and Buy",
            TradeType::Buy => "Buy",
            TradeType::Sell => "Sell",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SwqosType {
    Jito,
    NextBlock,
    ZeroSlot,
    Temporal,
    Bloxroute,
    Default,
}

pub type SwqosClient = dyn SwqosClientTrait + Send + Sync + 'static;

#[async_trait::async_trait]
pub trait SwqosClientTrait {
    async fn send_transaction(&self, trade_type: TradeType, transaction: &VersionedTransaction) -> Result<()>;
    async fn send_transactions(&self, trade_type: TradeType, transactions: &Vec<VersionedTransaction>) -> Result<()>;
    fn get_tip_account(&self) -> Result<String>;
    fn get_swqos_type(&self) -> SwqosType;
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SwqosRegion {
    NewYork,
    Frankfurt,
    Amsterdam,
    SLC,
    Tokyo,
    London,
    LosAngeles,
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SwqosConfig {
    Default(String),
    Jito(SwqosRegion),
    NextBlock(String, SwqosRegion),
    Bloxroute(String, SwqosRegion),
    Temporal(String, SwqosRegion),
    ZeroSlot(String, SwqosRegion),
}

impl SwqosConfig {
    pub fn get_endpoint(swqos_type: SwqosType, region: SwqosRegion) -> String {
        match swqos_type {
            SwqosType::Jito => SWQOS_ENDPOINTS_JITO[region as usize].to_string(),
            SwqosType::NextBlock => SWQOS_ENDPOINTS_NEXTBLOCK[region as usize].to_string(),
            SwqosType::ZeroSlot => SWQOS_ENDPOINTS_ZERO_SLOT[region as usize].to_string(),
            SwqosType::Temporal => SWQOS_ENDPOINTS_TEMPORAL[region as usize].to_string(),
            SwqosType::Bloxroute => SWQOS_ENDPOINTS_BLOX[region as usize].to_string(),
            SwqosType::Default => "".to_string(),
        }
    }

    pub fn get_swqos_client(rpc_url: String, commitment: CommitmentConfig, swqos_config: SwqosConfig) -> Arc<SwqosClient> {
        match swqos_config {
            SwqosConfig::Jito(region) => {
                let endpoint = SwqosConfig::get_endpoint(SwqosType::Jito, region);
                let jito_client = JitoClient::new(
                    rpc_url.clone(),
                    endpoint,
                    "".to_string()
                );
                Arc::new(jito_client)
            }
            SwqosConfig::NextBlock(auth_token, region) => {
                let endpoint = SwqosConfig::get_endpoint(SwqosType::NextBlock, region);
                let nextblock_client = NextBlockClient::new(
                    rpc_url.clone(),
                    endpoint.to_string(),
                    auth_token
                );
                Arc::new(nextblock_client)
            },
            SwqosConfig::ZeroSlot(auth_token, region) => {
                let endpoint = SwqosConfig::get_endpoint(SwqosType::ZeroSlot, region);
                let zeroslot_client = ZeroSlotClient::new(
                    rpc_url.clone(),
                    endpoint.to_string(),
                    auth_token
                );
                Arc::new(zeroslot_client)
            },
            SwqosConfig::Temporal(auth_token, region) => {  
                let endpoint = SwqosConfig::get_endpoint(SwqosType::Temporal, region);
                let temporal_client = TemporalClient::new(
                    rpc_url.clone(),
                    endpoint.to_string(),
                    auth_token
                );
                Arc::new(temporal_client)
            },
            SwqosConfig::Bloxroute(auth_token, region) => { 
                let endpoint = SwqosConfig::get_endpoint(SwqosType::Bloxroute, region);
                let bloxroute_client = BloxrouteClient::new(
                    rpc_url.clone(),
                    endpoint.to_string(),
                    auth_token
                );
                Arc::new(bloxroute_client)
            },
            SwqosConfig::Default(endpoint) => {
                let rpc = SolanaRpcClient::new_with_commitment(
                    endpoint,
                    commitment
                );   
                let rpc_client = SolRpcClient::new(Arc::new(rpc));
                Arc::new(rpc_client)
            }
        }
    }
}