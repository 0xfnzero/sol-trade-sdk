pub mod common;
pub mod solana_rpc;
pub mod jito;
pub mod nextblock;
pub mod zeroslot;
pub mod temporal;
pub mod bloxroute;
pub mod node1;
pub mod flashblock;
pub mod blockrazor;
pub mod astralane;
pub mod settings;

use std::sync::Arc;

use solana_sdk::{commitment_config::CommitmentConfig, transaction::VersionedTransaction};
use tokio::sync::RwLock;

use anyhow::Result;

use crate::{
    common::SolanaRpcClient, 
    constants::swqos::{
        SWQOS_ENDPOINTS_BLOX, 
        SWQOS_ENDPOINTS_JITO, 
        SWQOS_ENDPOINTS_NEXTBLOCK, 
        SWQOS_ENDPOINTS_TEMPORAL, 
        SWQOS_ENDPOINTS_ZERO_SLOT, 
        SWQOS_ENDPOINTS_NODE1, 
        SWQOS_ENDPOINTS_FLASHBLOCK,
        SWQOS_ENDPOINTS_BLOCKRAZOR,
        SWQOS_ENDPOINTS_ASTRALANE
    }, 
    swqos::{
        bloxroute::BloxrouteClient, 
        jito::JitoClient, 
        nextblock::NextBlockClient, 
        solana_rpc::SolRpcClient, 
        temporal::TemporalClient, 
        zeroslot::ZeroSlotClient, 
        node1::Node1Client, 
        flashblock::FlashBlockClient,
        blockrazor::BlockRazorClient,
        astralane::AstralaneClient
    }
};

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
    Node1,
    FlashBlock,
    BlockRazor,
    Astralane,
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
    /// Jito(uuid, region, custom_url)
    Jito(String, SwqosRegion, Option<String>),
    /// NextBlock(api_token, region, custom_url)
    NextBlock(String, SwqosRegion, Option<String>),
    /// Bloxroute(api_token, region, custom_url)
    Bloxroute(String, SwqosRegion, Option<String>),
    /// Temporal(api_token, region, custom_url)
    Temporal(String, SwqosRegion, Option<String>),
    /// ZeroSlot(api_token, region, custom_url)
    ZeroSlot(String, SwqosRegion, Option<String>),
    /// Node1(api_token, region, custom_url)
    Node1(String, SwqosRegion, Option<String>),
    /// FlashBlock(api_token, region, custom_url)
    FlashBlock(String, SwqosRegion, Option<String>),
    /// BlockRazor(api_token, region, custom_url)
    BlockRazor(String, SwqosRegion, Option<String>),
    /// Astralane(api_token, region, custom_url)
    Astralane(String, SwqosRegion, Option<String>),
}

impl SwqosConfig {
    pub fn get_endpoint(swqos_type: SwqosType, region: SwqosRegion, url: Option<String>) -> String {
        if let Some(custom_url) = url {
            return custom_url;
        }
        
        match swqos_type {
            SwqosType::Jito => SWQOS_ENDPOINTS_JITO[region as usize].to_string(),
            SwqosType::NextBlock => SWQOS_ENDPOINTS_NEXTBLOCK[region as usize].to_string(),
            SwqosType::ZeroSlot => SWQOS_ENDPOINTS_ZERO_SLOT[region as usize].to_string(),
            SwqosType::Temporal => SWQOS_ENDPOINTS_TEMPORAL[region as usize].to_string(),
            SwqosType::Bloxroute => SWQOS_ENDPOINTS_BLOX[region as usize].to_string(),
            SwqosType::Node1 => SWQOS_ENDPOINTS_NODE1[region as usize].to_string(),
            SwqosType::FlashBlock => SWQOS_ENDPOINTS_FLASHBLOCK[region as usize].to_string(),
            SwqosType::BlockRazor => SWQOS_ENDPOINTS_BLOCKRAZOR[region as usize].to_string(),
            SwqosType::Astralane => SWQOS_ENDPOINTS_ASTRALANE[region as usize].to_string(),
            SwqosType::Default => "".to_string(),
        }
    }

    pub fn get_swqos_client(rpc_url: String, commitment: CommitmentConfig, swqos_config: SwqosConfig) -> Arc<SwqosClient> {
        match swqos_config {
            SwqosConfig::Jito(auth_token, region, url) => {
                let endpoint = SwqosConfig::get_endpoint(SwqosType::Jito, region, url);
                let jito_client = JitoClient::new(
                    rpc_url.clone(),
                    endpoint,
                    auth_token
                );
                Arc::new(jito_client)
            }
            SwqosConfig::NextBlock(auth_token, region, url) => {
                let endpoint = SwqosConfig::get_endpoint(SwqosType::NextBlock, region, url);
                let nextblock_client = NextBlockClient::new(
                    rpc_url.clone(),
                    endpoint.to_string(),
                    auth_token
                );
                Arc::new(nextblock_client)
            },
            SwqosConfig::ZeroSlot(auth_token, region, url) => {
                let endpoint = SwqosConfig::get_endpoint(SwqosType::ZeroSlot, region, url);
                let zeroslot_client = ZeroSlotClient::new(
                    rpc_url.clone(),
                    endpoint.to_string(),
                    auth_token
                );
                Arc::new(zeroslot_client)
            },
            SwqosConfig::Temporal(auth_token, region, url) => {  
                let endpoint = SwqosConfig::get_endpoint(SwqosType::Temporal, region, url);
                let temporal_client = TemporalClient::new(
                    rpc_url.clone(),
                    endpoint.to_string(),
                    auth_token
                );
                Arc::new(temporal_client)
            },
            SwqosConfig::Bloxroute(auth_token, region, url) => { 
                let endpoint = SwqosConfig::get_endpoint(SwqosType::Bloxroute, region, url);
                let bloxroute_client = BloxrouteClient::new(
                    rpc_url.clone(),
                    endpoint.to_string(),
                    auth_token
                );
                Arc::new(bloxroute_client)
            },
            SwqosConfig::Node1(auth_token, region, url) => {
                let endpoint = SwqosConfig::get_endpoint(SwqosType::Node1, region, url);
                let node1_client = Node1Client::new(
                    rpc_url.clone(),
                    endpoint.to_string(),
                    auth_token
                );
                Arc::new(node1_client)
            },
            SwqosConfig::FlashBlock(auth_token, region, url) => {
                let endpoint = SwqosConfig::get_endpoint(SwqosType::FlashBlock, region, url);
                let flashblock_client = FlashBlockClient::new(
                    rpc_url.clone(),
                    endpoint.to_string(),
                    auth_token
                );
                Arc::new(flashblock_client)
            },
            SwqosConfig::BlockRazor(auth_token, region, url) => {
                let endpoint = SwqosConfig::get_endpoint(SwqosType::BlockRazor, region, url);
                let blockrazor_client = BlockRazorClient::new(
                    rpc_url.clone(),
                    endpoint.to_string(),
                    auth_token
                );
                Arc::new(blockrazor_client)
            },
            SwqosConfig::Astralane(auth_token, region, url) => {
                let endpoint = SwqosConfig::get_endpoint(SwqosType::Astralane, region, url);
                let astralane_client = AstralaneClient::new(
                    rpc_url.clone(),
                    endpoint.to_string(),
                    auth_token
                );
                Arc::new(astralane_client)
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