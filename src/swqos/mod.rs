pub mod astralane;
pub mod astralane_quic;
pub mod blockrazor;
pub mod bloxroute;
pub mod common;
pub mod flashblock;
pub mod glaive;
pub mod glaive_quic;
pub mod helius;
pub mod jito;
pub mod lightspeed;
pub mod lunarlander;
pub mod nextblock;
pub mod node1;
pub mod node1_quic;
pub mod serialization;
pub mod solami;
pub mod solana_rpc;
pub mod soyas;
pub mod speedlanding;
pub mod stellium;
pub mod temporal;
mod temporal_quic;
pub mod zeroslot;

use std::sync::Arc;

use solana_commitment_config::CommitmentConfig;
use solana_sdk::transaction::VersionedTransaction;

use anyhow::Result;

use crate::{
    common::SolanaRpcClient,
    constants::swqos::{
        SWQOS_ENDPOINTS_ASTRALANE_BINARY, SWQOS_ENDPOINTS_ASTRALANE_PLAIN,
        SWQOS_ENDPOINTS_ASTRALANE_QUIC, SWQOS_ENDPOINTS_ASTRALANE_QUIC_MEV,
        SWQOS_ENDPOINTS_BLOCKRAZOR, SWQOS_ENDPOINTS_BLOCKRAZOR_GRPC, SWQOS_ENDPOINTS_BLOX,
        SWQOS_ENDPOINTS_FLASHBLOCK, SWQOS_ENDPOINTS_GLAIVE, SWQOS_ENDPOINTS_GLAIVE_QUIC,
        SWQOS_ENDPOINTS_HELIUS, SWQOS_ENDPOINTS_JITO, SWQOS_ENDPOINTS_LUNARLANDER,
        SWQOS_ENDPOINTS_LUNARLANDER_QUIC, SWQOS_ENDPOINTS_NEXTBLOCK, SWQOS_ENDPOINTS_NODE1,
        SWQOS_ENDPOINTS_NODE1_QUIC, SWQOS_ENDPOINTS_SOLAMI, SWQOS_ENDPOINTS_SOYAS,
        SWQOS_ENDPOINTS_SPEEDLANDING, SWQOS_ENDPOINTS_STELLIUM, SWQOS_ENDPOINTS_TEMPORAL,
        SWQOS_ENDPOINTS_ZERO_SLOT, SWQOS_MIN_TIP_ASTRALANE, SWQOS_MIN_TIP_BLOCKRAZOR,
        SWQOS_MIN_TIP_BLOXROUTE, SWQOS_MIN_TIP_DEFAULT, SWQOS_MIN_TIP_FLASHBLOCK,
        SWQOS_MIN_TIP_GLAIVE, SWQOS_MIN_TIP_HELIUS, SWQOS_MIN_TIP_JITO, SWQOS_MIN_TIP_LIGHTSPEED,
        SWQOS_MIN_TIP_LUNARLANDER, SWQOS_MIN_TIP_NEXTBLOCK, SWQOS_MIN_TIP_NODE1,
        SWQOS_MIN_TIP_SOLAMI, SWQOS_MIN_TIP_SOYAS, SWQOS_MIN_TIP_SPEEDLANDING,
        SWQOS_MIN_TIP_STELLIUM, SWQOS_MIN_TIP_TEMPORAL, SWQOS_MIN_TIP_ZERO_SLOT,
    },
    swqos::{
        astralane::AstralaneClient, blockrazor::BlockRazorClient, bloxroute::BloxrouteClient,
        flashblock::FlashBlockClient, glaive::GlaiveClient, helius::HeliusClient, jito::JitoClient,
        lightspeed::LightspeedClient, lunarlander::LunarLanderClient, nextblock::NextBlockClient,
        node1::Node1Client, node1_quic::Node1QuicClient, solami::SolamiClient,
        solana_rpc::SolRpcClient, soyas::SoyasClient, speedlanding::SpeedlandingClient,
        stellium::StelliumClient, temporal::TemporalClient, zeroslot::ZeroSlotClient,
    },
};

// Tip 账户：`SwqosClient::get_tip_account()` 在各实现里多为静态常量；同一批多路提交时，
// 在 `trading::core::async_executor::execute_parallel` 内用局部 `tip_cache`（按 client 指针）去重解析。

/// SWQOS provider blacklist configuration
/// Providers added here will be disabled even if configured by user
/// To enable a provider, remove it from this list
pub const SWQOS_BLACKLIST: &[SwqosType] = &[
    SwqosType::NextBlock, // NextBlock is disabled by default
];

/// SWQOS 提交通道：HTTP、gRPC 或 QUIC（低延迟）。
/// BlockRazor 支持 gRPC 和 HTTP。
/// Node1、Glaive 与 Lunar Lander 支持 QUIC。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SwqosTransport {
    #[default]
    Http,
    Grpc,
    Quic,
}

/// Astralane 三种提交方式：QUIC TPU、Plain HTTP（`/iris`）、Binary HTTP（`/irisb` + bincode）。
/// 与全局 [`crate::common::TradeConfig::mev_protection`] 配合：HTTP 加 `mev-protect=true`；QUIC 选 `:9000` / `:7000`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AstralaneTransport {
    /// Binary over HTTP：`…/irisb?api-key=…&method=sendTransaction`（与 `AstralaneClient` 当前序列化一致）。
    Binary,
    /// Plain HTTP：`…/iris?…`（非 irisb 路径）。
    Plain,
    /// QUIC（`host:7000`；MEV 时 `host:9000`）。
    #[default]
    Quic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    Stellium,
    Lightspeed,
    Soyas,
    Speedlanding,
    Helius,
    Solami,
    LunarLander,
    Glaive,
    Default,
}

impl SwqosType {
    /// Label for log alignment; same as Debug output (e.g. "Soyas", "Speedlanding").
    #[inline]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Jito => "Jito",
            Self::NextBlock => "NextBlock",
            Self::ZeroSlot => "ZeroSlot",
            Self::Temporal => "Temporal",
            Self::Bloxroute => "Bloxroute",
            Self::Node1 => "Node1",
            Self::FlashBlock => "FlashBlock",
            Self::BlockRazor => "BlockRazor",
            Self::Astralane => "Astralane",
            Self::Stellium => "Stellium",
            Self::Lightspeed => "Lightspeed",
            Self::Soyas => "Soyas",
            Self::Speedlanding => "Speedlanding",
            Self::Helius => "Helius",
            Self::Solami => "Solami",
            Self::LunarLander => "LunarLander",
            Self::Glaive => "Glaive",
            Self::Default => "Default",
        }
    }

    pub fn values() -> Vec<Self> {
        vec![
            Self::Jito,
            Self::NextBlock,
            Self::ZeroSlot,
            Self::Temporal,
            Self::Bloxroute,
            Self::Node1,
            Self::FlashBlock,
            Self::BlockRazor,
            Self::Astralane,
            Self::Stellium,
            Self::Lightspeed,
            Self::Soyas,
            Self::Speedlanding,
            Self::Helius,
            Self::Solami,
            Self::LunarLander,
            Self::Glaive,
            Self::Default,
        ]
    }
}

pub type SwqosClient = dyn SwqosClientTrait + Send + Sync + 'static;

#[async_trait::async_trait]
pub trait SwqosClientTrait {
    /// User-configured minimum tip in lamports; None preserves existing filtering behavior.
    fn configured_min_tip_lamports(&self) -> Option<u64> {
        None
    }

    async fn send_transaction(
        &self,
        trade_type: TradeType,
        transaction: &VersionedTransaction,
        wait_confirmation: bool,
    ) -> Result<()>;
    async fn send_transactions(
        &self,
        trade_type: TradeType,
        transactions: &Vec<VersionedTransaction>,
        wait_confirmation: bool,
    ) -> Result<()>;
    fn get_tip_account(&self) -> Result<String>;
    fn get_swqos_type(&self) -> SwqosType;
    /// Minimum tip in SOL required by this provider. Helius returns lower value when swqos_only is true.
    #[inline]
    fn min_tip_sol(&self) -> f64 {
        match self.get_swqos_type() {
            SwqosType::Jito => SWQOS_MIN_TIP_JITO,
            SwqosType::NextBlock => SWQOS_MIN_TIP_NEXTBLOCK,
            SwqosType::ZeroSlot => SWQOS_MIN_TIP_ZERO_SLOT,
            SwqosType::Temporal => SWQOS_MIN_TIP_TEMPORAL,
            SwqosType::Bloxroute => SWQOS_MIN_TIP_BLOXROUTE,
            SwqosType::Node1 => SWQOS_MIN_TIP_NODE1,
            SwqosType::FlashBlock => SWQOS_MIN_TIP_FLASHBLOCK,
            SwqosType::BlockRazor => SWQOS_MIN_TIP_BLOCKRAZOR,
            SwqosType::Astralane => SWQOS_MIN_TIP_ASTRALANE,
            SwqosType::Stellium => SWQOS_MIN_TIP_STELLIUM,
            SwqosType::Lightspeed => SWQOS_MIN_TIP_LIGHTSPEED,
            SwqosType::Soyas => SWQOS_MIN_TIP_SOYAS,
            SwqosType::Speedlanding => SWQOS_MIN_TIP_SPEEDLANDING,
            SwqosType::Helius => SWQOS_MIN_TIP_HELIUS,
            SwqosType::Solami => SWQOS_MIN_TIP_SOLAMI,
            SwqosType::LunarLander => SWQOS_MIN_TIP_LUNARLANDER,
            SwqosType::Glaive => SWQOS_MIN_TIP_GLAIVE,
            SwqosType::Default => SWQOS_MIN_TIP_DEFAULT,
        }
    }
}

/// Preserve provider transport, built-in limits and fallback behavior while attaching a route limit.
struct MinTipSwqosClient {
    inner: Arc<SwqosClient>,
    min_tip_lamports: u64,
}

#[async_trait::async_trait]
impl SwqosClientTrait for MinTipSwqosClient {
    fn configured_min_tip_lamports(&self) -> Option<u64> {
        Some(self.min_tip_lamports)
    }
    async fn send_transaction(
        &self,
        trade_type: TradeType,
        transaction: &VersionedTransaction,
        wait_confirmation: bool,
    ) -> Result<()> {
        self.inner.send_transaction(trade_type, transaction, wait_confirmation).await
    }
    async fn send_transactions(
        &self,
        trade_type: TradeType,
        transactions: &Vec<VersionedTransaction>,
        wait_confirmation: bool,
    ) -> Result<()> {
        self.inner.send_transactions(trade_type, transactions, wait_confirmation).await
    }
    fn get_tip_account(&self) -> Result<String> {
        self.inner.get_tip_account()
    }
    fn get_swqos_type(&self) -> SwqosType {
        self.inner.get_swqos_type()
    }
    fn min_tip_sol(&self) -> f64 {
        self.inner.min_tip_sol()
    }
}

/// 地理区域，用于默认 SWQOS 端点下标（见 `constants::swqos`）。
///
/// 各服务商常量表在**缺独立 PoP**时，于**已公布的端点集合内**按地理距离选最近项；[`SwqosRegion::Default`] 不表示地球上的位置，表中为全局/枢纽回退，不适用地理就近。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SwqosRegion {
    NewYork,
    Frankfurt,
    Amsterdam,
    /// Ireland (EU); Jito publishes `dublin.mainnet.block-engine.jito.wtf`.
    Dublin,
    SLC,
    Tokyo,
    /// Southeast Asia (Singapore); not interchangeable with [`SwqosRegion::Tokyo`].
    Singapore,
    London,
    LosAngeles,
    /// 非地理区域：未指定区域时的回退，对应表中全局 URL 或枢纽，**不按地理距离选取**。
    Default,
}

/// Provider configurations end with an optional minimum tip in SOL (e.g. Some(0.0001)).
/// None adds no filtering; explicit limits apply before building or sending the route transaction.
#[derive(Debug, Clone)]
pub enum SwqosConfig {
    Default(String),
    /// Jito(uuid, region, custom_url, min_tip_sol)
    Jito(String, SwqosRegion, Option<String>, Option<f64>),
    /// NextBlock(api_token, region, custom_url, min_tip_sol)
    NextBlock(String, SwqosRegion, Option<String>, Option<f64>),
    /// Bloxroute(api_token, region, custom_url, min_tip_sol)
    Bloxroute(String, SwqosRegion, Option<String>, Option<f64>),
    /// Temporal(api_token, region, custom_url, min_tip_sol). Without a custom URL, uses QUIC/H3 then HTTP Batch fallback.
    /// A custom URL remains an explicit HTTP Binary Batch endpoint.
    Temporal(String, SwqosRegion, Option<String>, Option<f64>),
    /// ZeroSlot(api_token, region, custom_url, min_tip_sol)
    ZeroSlot(String, SwqosRegion, Option<String>, Option<f64>),
    /// Node1(api_token, region, custom_url, transport, min_tip_sol). transport=None => HTTP; Some(Quic) => QUIC (port 16666, UUID auth).
    Node1(String, SwqosRegion, Option<String>, Option<SwqosTransport>, Option<f64>),
    /// FlashBlock(api_token, region, custom_url, min_tip_sol)
    FlashBlock(String, SwqosRegion, Option<String>, Option<f64>),
    /// BlockRazor(api_token, region, custom_url, transport, min_tip_sol). transport=None 或 Grpc => gRPC; Some(Http) => HTTP.
    BlockRazor(String, SwqosRegion, Option<String>, Option<SwqosTransport>, Option<f64>),
    /// Astralane(api_token, region, custom_url, mode, min_tip_sol). `None` => QUIC then Binary HTTP fallback.
    /// A custom URL with no mode remains an explicit Binary HTTP endpoint.
    Astralane(String, SwqosRegion, Option<String>, Option<AstralaneTransport>, Option<f64>),
    /// Stellium(api_token, region, custom_url, min_tip_sol)
    Stellium(String, SwqosRegion, Option<String>, Option<f64>),
    /// Lightspeed(api_key, region, custom_url, min_tip_sol) - Solana Vibe Station
    /// Endpoint format: https://<tier>.rpc.solanavibestation.com/lightspeed?api_key=<key>
    /// Minimum tip: 0.001 SOL
    Lightspeed(String, SwqosRegion, Option<String>, Option<f64>),
    /// Soyas(api_token, region, custom_url, min_tip_sol)
    Soyas(String, SwqosRegion, Option<String>, Option<f64>),
    /// To apply for an API key, please contact -> https://t.me/speedlanding_bot?start=0xzero
    /// Minimum tip: 0.001 SOL
    Speedlanding(String, SwqosRegion, Option<String>, Option<f64>),
    /// Helius Sender: dual routing to validators and Jito. API key optional (custom TPS only).
    /// (api_key, region, custom_url, swqos_only, min_tip_sol). swqos_only: None => false (min tip 0.0002 SOL); Some(true) => SWQOS-only (min tip 0.000005 SOL, much lower).
    Helius(String, SwqosRegion, Option<String>, Option<bool>, Option<f64>),
    /// Solami(api_key, region, custom_url, min_tip_sol)
    Solami(String, SwqosRegion, Option<String>, Option<f64>),
    /// Lunar Lander (HelloMoon): binary tx via QUIC (port 16888) or HTTP POST /send-bin.
    /// (api_key, region, custom_url, transport, min_tip_sol). transport=None => QUIC; Some(Http) => HTTP.
    /// Minimum tip: 0.001 SOL. Apply for API key: https://docs.hellomoon.io/reference/lunar-lander
    LunarLander(String, SwqosRegion, Option<String>, Option<SwqosTransport>, Option<f64>),
    /// Glaive(api_key_uuid, region, custom_url, transport, min_tip_sol).
    /// transport=None => QUIC (official lowest-latency path, UDP/4000); Some(Http) => binary HTTP.
    /// Minimum tip: 0.0001 SOL. API and protocol docs: <https://glaive.trade/docs>
    Glaive(String, SwqosRegion, Option<String>, Option<SwqosTransport>, Option<f64>),
}

// Configuration identity includes the exact float bits. This remains reflexive even
// for invalid NaN values, which are rejected when constructing a provider client.
impl PartialEq for SwqosConfig {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Default(a), Self::Default(b)) => a == b,
            (Self::Jito(a0, a1, a2, am), Self::Jito(b0, b1, b2, bm)) => {
                (a0, a1, a2, am.map(f64::to_bits)) == (b0, b1, b2, bm.map(f64::to_bits))
            }
            (Self::NextBlock(a0, a1, a2, am), Self::NextBlock(b0, b1, b2, bm)) => {
                (a0, a1, a2, am.map(f64::to_bits)) == (b0, b1, b2, bm.map(f64::to_bits))
            }
            (Self::Bloxroute(a0, a1, a2, am), Self::Bloxroute(b0, b1, b2, bm)) => {
                (a0, a1, a2, am.map(f64::to_bits)) == (b0, b1, b2, bm.map(f64::to_bits))
            }
            (Self::Temporal(a0, a1, a2, am), Self::Temporal(b0, b1, b2, bm)) => {
                (a0, a1, a2, am.map(f64::to_bits)) == (b0, b1, b2, bm.map(f64::to_bits))
            }
            (Self::ZeroSlot(a0, a1, a2, am), Self::ZeroSlot(b0, b1, b2, bm)) => {
                (a0, a1, a2, am.map(f64::to_bits)) == (b0, b1, b2, bm.map(f64::to_bits))
            }
            (Self::Node1(a0, a1, a2, a3, am), Self::Node1(b0, b1, b2, b3, bm)) => {
                (a0, a1, a2, a3, am.map(f64::to_bits)) == (b0, b1, b2, b3, bm.map(f64::to_bits))
            }
            (Self::FlashBlock(a0, a1, a2, am), Self::FlashBlock(b0, b1, b2, bm)) => {
                (a0, a1, a2, am.map(f64::to_bits)) == (b0, b1, b2, bm.map(f64::to_bits))
            }
            (Self::BlockRazor(a0, a1, a2, a3, am), Self::BlockRazor(b0, b1, b2, b3, bm)) => {
                (a0, a1, a2, a3, am.map(f64::to_bits)) == (b0, b1, b2, b3, bm.map(f64::to_bits))
            }
            (Self::Astralane(a0, a1, a2, a3, am), Self::Astralane(b0, b1, b2, b3, bm)) => {
                (a0, a1, a2, a3, am.map(f64::to_bits)) == (b0, b1, b2, b3, bm.map(f64::to_bits))
            }
            (Self::Stellium(a0, a1, a2, am), Self::Stellium(b0, b1, b2, bm)) => {
                (a0, a1, a2, am.map(f64::to_bits)) == (b0, b1, b2, bm.map(f64::to_bits))
            }
            (Self::Lightspeed(a0, a1, a2, am), Self::Lightspeed(b0, b1, b2, bm)) => {
                (a0, a1, a2, am.map(f64::to_bits)) == (b0, b1, b2, bm.map(f64::to_bits))
            }
            (Self::Soyas(a0, a1, a2, am), Self::Soyas(b0, b1, b2, bm)) => {
                (a0, a1, a2, am.map(f64::to_bits)) == (b0, b1, b2, bm.map(f64::to_bits))
            }
            (Self::Speedlanding(a0, a1, a2, am), Self::Speedlanding(b0, b1, b2, bm)) => {
                (a0, a1, a2, am.map(f64::to_bits)) == (b0, b1, b2, bm.map(f64::to_bits))
            }
            (Self::Helius(a0, a1, a2, a3, am), Self::Helius(b0, b1, b2, b3, bm)) => {
                (a0, a1, a2, a3, am.map(f64::to_bits)) == (b0, b1, b2, b3, bm.map(f64::to_bits))
            }
            (Self::Solami(a0, a1, a2, am), Self::Solami(b0, b1, b2, bm)) => {
                (a0, a1, a2, am.map(f64::to_bits)) == (b0, b1, b2, bm.map(f64::to_bits))
            }
            (Self::LunarLander(a0, a1, a2, a3, am), Self::LunarLander(b0, b1, b2, b3, bm)) => {
                (a0, a1, a2, a3, am.map(f64::to_bits)) == (b0, b1, b2, b3, bm.map(f64::to_bits))
            }
            (Self::Glaive(a0, a1, a2, a3, am), Self::Glaive(b0, b1, b2, b3, bm)) => {
                (a0, a1, a2, a3, am.map(f64::to_bits)) == (b0, b1, b2, b3, bm.map(f64::to_bits))
            }
            _ => false,
        }
    }
}
impl Eq for SwqosConfig {}
impl std::hash::Hash for SwqosConfig {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::Default(url) => url.hash(state),
            Self::Jito(a0, a1, a2, minimum) => (a0, a1, a2, minimum.map(f64::to_bits)).hash(state),
            Self::NextBlock(a0, a1, a2, minimum) => {
                (a0, a1, a2, minimum.map(f64::to_bits)).hash(state)
            }
            Self::Bloxroute(a0, a1, a2, minimum) => {
                (a0, a1, a2, minimum.map(f64::to_bits)).hash(state)
            }
            Self::Temporal(a0, a1, a2, minimum) => {
                (a0, a1, a2, minimum.map(f64::to_bits)).hash(state)
            }
            Self::ZeroSlot(a0, a1, a2, minimum) => {
                (a0, a1, a2, minimum.map(f64::to_bits)).hash(state)
            }
            Self::Node1(a0, a1, a2, a3, minimum) => {
                (a0, a1, a2, a3, minimum.map(f64::to_bits)).hash(state)
            }
            Self::FlashBlock(a0, a1, a2, minimum) => {
                (a0, a1, a2, minimum.map(f64::to_bits)).hash(state)
            }
            Self::BlockRazor(a0, a1, a2, a3, minimum) => {
                (a0, a1, a2, a3, minimum.map(f64::to_bits)).hash(state)
            }
            Self::Astralane(a0, a1, a2, a3, minimum) => {
                (a0, a1, a2, a3, minimum.map(f64::to_bits)).hash(state)
            }
            Self::Stellium(a0, a1, a2, minimum) => {
                (a0, a1, a2, minimum.map(f64::to_bits)).hash(state)
            }
            Self::Lightspeed(a0, a1, a2, minimum) => {
                (a0, a1, a2, minimum.map(f64::to_bits)).hash(state)
            }
            Self::Soyas(a0, a1, a2, minimum) => (a0, a1, a2, minimum.map(f64::to_bits)).hash(state),
            Self::Speedlanding(a0, a1, a2, minimum) => {
                (a0, a1, a2, minimum.map(f64::to_bits)).hash(state)
            }
            Self::Helius(a0, a1, a2, a3, minimum) => {
                (a0, a1, a2, a3, minimum.map(f64::to_bits)).hash(state)
            }
            Self::Solami(a0, a1, a2, minimum) => {
                (a0, a1, a2, minimum.map(f64::to_bits)).hash(state)
            }
            Self::LunarLander(a0, a1, a2, a3, minimum) => {
                (a0, a1, a2, a3, minimum.map(f64::to_bits)).hash(state)
            }
            Self::Glaive(a0, a1, a2, a3, minimum) => {
                (a0, a1, a2, a3, minimum.map(f64::to_bits)).hash(state)
            }
        }
    }
}

impl SwqosConfig {
    /// Explicit route minimum in SOL; None preserves existing filtering behavior.
    pub fn min_tip_sol(&self) -> Option<f64> {
        match self {
            Self::Default(_) => None,
            Self::Jito(_, _, _, minimum) => *minimum,
            Self::NextBlock(_, _, _, minimum) => *minimum,
            Self::Bloxroute(_, _, _, minimum) => *minimum,
            Self::Temporal(_, _, _, minimum) => *minimum,
            Self::ZeroSlot(_, _, _, minimum) => *minimum,
            Self::Node1(_, _, _, _, minimum) => *minimum,
            Self::FlashBlock(_, _, _, minimum) => *minimum,
            Self::BlockRazor(_, _, _, _, minimum) => *minimum,
            Self::Astralane(_, _, _, _, minimum) => *minimum,
            Self::Stellium(_, _, _, minimum) => *minimum,
            Self::Lightspeed(_, _, _, minimum) => *minimum,
            Self::Soyas(_, _, _, minimum) => *minimum,
            Self::Speedlanding(_, _, _, minimum) => *minimum,
            Self::Helius(_, _, _, _, minimum) => *minimum,
            Self::Solami(_, _, _, minimum) => *minimum,
            Self::LunarLander(_, _, _, _, minimum) => *minimum,
            Self::Glaive(_, _, _, _, minimum) => *minimum,
        }
    }

    fn min_tip_lamports(&self) -> Result<Option<u64>> {
        self.min_tip_sol()
            .map(|minimum| {
                anyhow::ensure!(
                    minimum.is_finite() && minimum >= 0.0,
                    "SWQOS min_tip must be a finite non-negative SOL amount"
                );
                let lamports = minimum * 1_000_000_000.0;
                anyhow::ensure!(
                    lamports < u64::MAX as f64,
                    "SWQOS min_tip exceeds the lamport range"
                );
                // Match transaction-builder rounding to the nearest whole lamport.
                Ok(lamports.round() as u64)
            })
            .transpose()
    }

    pub fn swqos_type(&self) -> SwqosType {
        match self {
            SwqosConfig::Default(_) => SwqosType::Default,
            SwqosConfig::Jito(_, _, _, _) => SwqosType::Jito,
            SwqosConfig::NextBlock(_, _, _, _) => SwqosType::NextBlock,
            SwqosConfig::Bloxroute(_, _, _, _) => SwqosType::Bloxroute,
            SwqosConfig::Temporal(_, _, _, _) => SwqosType::Temporal,
            SwqosConfig::ZeroSlot(_, _, _, _) => SwqosType::ZeroSlot,
            SwqosConfig::Node1(_, _, _, _, _) => SwqosType::Node1,
            SwqosConfig::FlashBlock(_, _, _, _) => SwqosType::FlashBlock,
            SwqosConfig::BlockRazor(_, _, _, _, _) => SwqosType::BlockRazor,
            SwqosConfig::Astralane(_, _, _, _, _) => SwqosType::Astralane,
            SwqosConfig::Stellium(_, _, _, _) => SwqosType::Stellium,
            SwqosConfig::Lightspeed(_, _, _, _) => SwqosType::Lightspeed,
            SwqosConfig::Soyas(_, _, _, _) => SwqosType::Soyas,
            SwqosConfig::Speedlanding(_, _, _, _) => SwqosType::Speedlanding,
            SwqosConfig::Helius(_, _, _, _, _) => SwqosType::Helius,
            SwqosConfig::Solami(_, _, _, _) => SwqosType::Solami,
            SwqosConfig::LunarLander(_, _, _, _, _) => SwqosType::LunarLander,
            SwqosConfig::Glaive(_, _, _, _, _) => SwqosType::Glaive,
        }
    }

    /// Check if current config is in the blacklist
    pub fn is_blacklisted(&self) -> bool {
        SWQOS_BLACKLIST.contains(&self.swqos_type())
    }

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
            SwqosType::Astralane => SWQOS_ENDPOINTS_ASTRALANE_BINARY[region as usize].to_string(),
            SwqosType::Stellium => SWQOS_ENDPOINTS_STELLIUM[region as usize].to_string(),
            SwqosType::Lightspeed => "".to_string(), // Lightspeed requires custom URL with api_key
            SwqosType::Soyas => SWQOS_ENDPOINTS_SOYAS[region as usize].to_string(),
            SwqosType::Speedlanding => SWQOS_ENDPOINTS_SPEEDLANDING[region as usize].to_string(),
            SwqosType::Helius => SWQOS_ENDPOINTS_HELIUS[region as usize].to_string(),
            SwqosType::Solami => SWQOS_ENDPOINTS_SOLAMI[region as usize].to_string(),
            SwqosType::LunarLander => SWQOS_ENDPOINTS_LUNARLANDER[region as usize].to_string(),
            SwqosType::Glaive => SWQOS_ENDPOINTS_GLAIVE[region as usize].to_string(),
            SwqosType::Default => "".to_string(),
        }
    }

    pub fn get_endpoint_with_transport(
        swqos_type: SwqosType,
        region: SwqosRegion,
        url: Option<String>,
        transport: Option<SwqosTransport>,
        _mev_protection: bool,
    ) -> String {
        if let Some(custom_url) = url {
            return custom_url;
        }

        match swqos_type {
            SwqosType::BlockRazor => {
                // transport=None 或 transport=Grpc => gRPC; transport=Http => HTTP
                let use_http = transport.map_or(false, |t| t == SwqosTransport::Http);
                if use_http {
                    SWQOS_ENDPOINTS_BLOCKRAZOR[region as usize].to_string()
                } else {
                    SWQOS_ENDPOINTS_BLOCKRAZOR_GRPC[region as usize].to_string()
                }
            }
            SwqosType::Node1 => {
                let use_quic = transport.map_or(false, |t| t == SwqosTransport::Quic);
                if use_quic {
                    SWQOS_ENDPOINTS_NODE1_QUIC[region as usize].to_string()
                } else {
                    SWQOS_ENDPOINTS_NODE1[region as usize].to_string()
                }
            }
            SwqosType::LunarLander => {
                let use_quic = transport.unwrap_or(SwqosTransport::Quic) == SwqosTransport::Quic;
                if use_quic {
                    SWQOS_ENDPOINTS_LUNARLANDER_QUIC[region as usize].to_string()
                } else {
                    SWQOS_ENDPOINTS_LUNARLANDER[region as usize].to_string()
                }
            }
            SwqosType::Glaive => {
                let use_quic = transport.unwrap_or(SwqosTransport::Quic) == SwqosTransport::Quic;
                if use_quic {
                    SWQOS_ENDPOINTS_GLAIVE_QUIC[region as usize].to_string()
                } else {
                    SWQOS_ENDPOINTS_GLAIVE[region as usize].to_string()
                }
            }
            _ => Self::get_endpoint(swqos_type, region, None),
        }
    }

    pub async fn get_swqos_client(
        rpc_url: String,
        commitment: CommitmentConfig,
        swqos_config: SwqosConfig,
        mev_protection: bool,
    ) -> Result<Arc<SwqosClient>> {
        let minimum = swqos_config.min_tip_lamports()?;
        let client =
            Self::build_swqos_client(rpc_url, commitment, swqos_config, mev_protection).await?;
        Ok(match minimum {
            Some(min_tip_lamports) => {
                Arc::new(MinTipSwqosClient { inner: client, min_tip_lamports })
            }
            None => client,
        })
    }

    async fn build_swqos_client(
        rpc_url: String,
        commitment: CommitmentConfig,
        swqos_config: SwqosConfig,
        mev_protection: bool,
    ) -> Result<Arc<SwqosClient>> {
        match swqos_config {
            SwqosConfig::Jito(auth_token, region, url, _) => {
                let endpoint = SwqosConfig::get_endpoint(SwqosType::Jito, region, url);
                let jito_client = JitoClient::new(rpc_url.clone(), endpoint, auth_token);
                Ok(Arc::new(jito_client))
            }
            SwqosConfig::NextBlock(auth_token, region, url, _) => {
                let endpoint = SwqosConfig::get_endpoint(SwqosType::NextBlock, region, url);
                let nextblock_client =
                    NextBlockClient::new(rpc_url.clone(), endpoint.to_string(), auth_token);
                Ok(Arc::new(nextblock_client))
            }
            SwqosConfig::ZeroSlot(auth_token, region, url, _) => {
                let endpoint = SwqosConfig::get_endpoint(SwqosType::ZeroSlot, region, url);
                let zeroslot_client =
                    ZeroSlotClient::new(rpc_url.clone(), endpoint.to_string(), auth_token);
                Ok(Arc::new(zeroslot_client))
            }
            SwqosConfig::Temporal(auth_token, region, url, _) => {
                if let Some(endpoint) = url {
                    return Ok(Arc::new(TemporalClient::new(
                        rpc_url.clone(),
                        endpoint,
                        auth_token,
                    )));
                }
                let endpoint = SWQOS_ENDPOINTS_TEMPORAL[region as usize].to_string();
                let temporal_client = TemporalClient::new_quic_with_fallback(
                    rpc_url.clone(),
                    endpoint.clone(),
                    auth_token.clone(),
                )
                .await
                .unwrap_or_else(|error| {
                    tracing::warn!(target: "sol_trade_sdk", "Temporal QUIC setup failed; using HTTP Batch only: {error}");
                    TemporalClient::new(rpc_url.clone(), endpoint, auth_token)
                });
                Ok(Arc::new(temporal_client))
            }
            SwqosConfig::Bloxroute(auth_token, region, url, _) => {
                let endpoint = SwqosConfig::get_endpoint(SwqosType::Bloxroute, region, url);
                let bloxroute_client =
                    BloxrouteClient::new(rpc_url.clone(), endpoint.to_string(), auth_token);
                Ok(Arc::new(bloxroute_client))
            }
            SwqosConfig::Node1(auth_token, region, url, transport, _) => {
                let use_quic = transport.map_or(false, |t| t == SwqosTransport::Quic);
                if use_quic {
                    let quic_endpoint = url
                        .unwrap_or_else(|| SWQOS_ENDPOINTS_NODE1_QUIC[region as usize].to_string());
                    let node1_quic =
                        Node1QuicClient::connect(&quic_endpoint, &auth_token, rpc_url.clone())
                            .await?;
                    Ok(Arc::new(node1_quic))
                } else {
                    let endpoint = SwqosConfig::get_endpoint(SwqosType::Node1, region, url);
                    let node1_client =
                        Node1Client::new(rpc_url.clone(), endpoint.to_string(), auth_token);
                    Ok(Arc::new(node1_client))
                }
            }
            SwqosConfig::FlashBlock(auth_token, region, url, _) => {
                let endpoint = SwqosConfig::get_endpoint(SwqosType::FlashBlock, region, url);
                let flashblock_client =
                    FlashBlockClient::new(rpc_url.clone(), endpoint.to_string(), auth_token);
                Ok(Arc::new(flashblock_client))
            }
            SwqosConfig::BlockRazor(auth_token, region, url, transport, _) => {
                let region_index = region as usize;
                if url.is_some() && transport.is_none() {
                    return Ok(Arc::new(BlockRazorClient::new_http(
                        rpc_url.clone(),
                        url.unwrap(),
                        auth_token,
                        mev_protection,
                    )));
                }
                match transport {
                    Some(SwqosTransport::Http) => {
                        let endpoint = url.unwrap_or_else(|| {
                            SWQOS_ENDPOINTS_BLOCKRAZOR[region_index].to_string()
                        });
                        Ok(Arc::new(BlockRazorClient::new_http(
                            rpc_url.clone(),
                            endpoint,
                            auth_token,
                            mev_protection,
                        )))
                    }
                    Some(SwqosTransport::Grpc) => {
                        let endpoint = url.unwrap_or_else(|| {
                            SWQOS_ENDPOINTS_BLOCKRAZOR_GRPC[region_index].to_string()
                        });
                        Ok(Arc::new(
                            BlockRazorClient::new_grpc(
                                rpc_url.clone(),
                                endpoint,
                                auth_token,
                                mev_protection,
                            )
                            .await?,
                        ))
                    }
                    Some(SwqosTransport::Quic) => {
                        anyhow::bail!(
                            "BlockRazor does not support the QUIC transaction-submission transport"
                        )
                    }
                    None => {
                        let grpc_endpoint =
                            SWQOS_ENDPOINTS_BLOCKRAZOR_GRPC[region_index].to_string();
                        let http_endpoint = SWQOS_ENDPOINTS_BLOCKRAZOR[region_index].to_string();
                        let client = BlockRazorClient::new_grpc_with_http_fallback(
                            rpc_url.clone(),
                            grpc_endpoint,
                            http_endpoint.clone(),
                            auth_token.clone(),
                            mev_protection,
                        )
                        .await
                        .unwrap_or_else(|error| {
                            tracing::warn!(target: "sol_trade_sdk", "BlockRazor gRPC setup failed; using HTTP only: {error}");
                            BlockRazorClient::new_http(
                                rpc_url.clone(),
                                http_endpoint,
                                auth_token,
                                mev_protection,
                            )
                        });
                        Ok(Arc::new(client))
                    }
                }
            }
            SwqosConfig::Astralane(auth_token, region, url, mode, _) => {
                let region_index = region as usize;
                if mode.is_none() {
                    if let Some(endpoint) = url {
                        return Ok(Arc::new(AstralaneClient::new(
                            rpc_url.clone(),
                            endpoint,
                            auth_token,
                            mev_protection,
                        )));
                    }
                    let quic_endpoint = if mev_protection {
                        SWQOS_ENDPOINTS_ASTRALANE_QUIC_MEV[region_index].to_string()
                    } else {
                        SWQOS_ENDPOINTS_ASTRALANE_QUIC[region_index].to_string()
                    };
                    let http_endpoint = SWQOS_ENDPOINTS_ASTRALANE_BINARY[region_index].to_string();
                    let client = AstralaneClient::new_quic_with_http_fallback(
                        rpc_url.clone(),
                        &quic_endpoint,
                        http_endpoint.clone(),
                        auth_token.clone(),
                        mev_protection,
                    )
                    .await
                    .unwrap_or_else(|error| {
                        tracing::warn!(target: "sol_trade_sdk", "Astralane QUIC setup failed; using Binary HTTP only: {error}");
                        AstralaneClient::new(
                            rpc_url.clone(),
                            http_endpoint,
                            auth_token,
                            mev_protection,
                        )
                    });
                    return Ok(Arc::new(client));
                }

                match mode.unwrap() {
                    AstralaneTransport::Quic => {
                        let quic_endpoint = url.unwrap_or_else(|| {
                            if mev_protection {
                                SWQOS_ENDPOINTS_ASTRALANE_QUIC_MEV[region_index].to_string()
                            } else {
                                SWQOS_ENDPOINTS_ASTRALANE_QUIC[region_index].to_string()
                            }
                        });
                        let astralane_client =
                            AstralaneClient::new_quic(rpc_url.clone(), &quic_endpoint, auth_token)
                                .await?;
                        Ok(Arc::new(astralane_client))
                    }
                    AstralaneTransport::Plain => {
                        let endpoint = url.unwrap_or_else(|| {
                            SWQOS_ENDPOINTS_ASTRALANE_PLAIN[region_index].to_string()
                        });
                        let astralane_client = AstralaneClient::new(
                            rpc_url.clone(),
                            endpoint,
                            auth_token,
                            mev_protection,
                        );
                        Ok(Arc::new(astralane_client))
                    }
                    AstralaneTransport::Binary => {
                        let endpoint = url.unwrap_or_else(|| {
                            SWQOS_ENDPOINTS_ASTRALANE_BINARY[region_index].to_string()
                        });
                        let astralane_client = AstralaneClient::new(
                            rpc_url.clone(),
                            endpoint,
                            auth_token,
                            mev_protection,
                        );
                        Ok(Arc::new(astralane_client))
                    }
                }
            }
            SwqosConfig::Stellium(auth_token, region, url, _) => {
                let endpoint = SwqosConfig::get_endpoint(SwqosType::Stellium, region, url);
                let stellium_client =
                    StelliumClient::new(rpc_url.clone(), endpoint.to_string(), auth_token);
                Ok(Arc::new(stellium_client))
            }
            SwqosConfig::Lightspeed(auth_token, region, url, _) => {
                let endpoint = SwqosConfig::get_endpoint(SwqosType::Lightspeed, region, url);
                let lightspeed_client =
                    LightspeedClient::new(rpc_url.clone(), endpoint.to_string(), auth_token);
                Ok(Arc::new(lightspeed_client))
            }
            SwqosConfig::Soyas(auth_token, region, url, _) => {
                let endpoint = SwqosConfig::get_endpoint(SwqosType::Soyas, region, url);
                let soyas_client =
                    SoyasClient::new(rpc_url.clone(), endpoint.to_string(), auth_token).await?;
                Ok(Arc::new(soyas_client))
            }
            SwqosConfig::Speedlanding(auth_token, region, url, _) => {
                let endpoint = SwqosConfig::get_endpoint(SwqosType::Speedlanding, region, url);
                let speedlanding_client =
                    SpeedlandingClient::new(rpc_url.clone(), endpoint.to_string(), auth_token)
                        .await?;
                Ok(Arc::new(speedlanding_client))
            }
            SwqosConfig::Helius(api_key, region, url, swqos_only, _) => {
                let swqos_only = swqos_only.unwrap_or(false);
                let endpoint = SwqosConfig::get_endpoint(SwqosType::Helius, region, url.clone());
                let api_key_opt = if api_key.is_empty() { None } else { Some(api_key.clone()) };
                let helius_client =
                    HeliusClient::new(rpc_url.clone(), endpoint, api_key_opt, swqos_only);
                Ok(Arc::new(helius_client))
            }
            SwqosConfig::Solami(auth_token, region, url, _) => {
                let endpoint = SwqosConfig::get_endpoint(SwqosType::Solami, region, url);
                let solami_client =
                    SolamiClient::new(rpc_url.clone(), endpoint.to_string(), auth_token).await?;
                Ok(Arc::new(solami_client))
            }
            SwqosConfig::LunarLander(api_key, region, url, transport, _) => {
                let use_quic = transport.unwrap_or(SwqosTransport::Quic) == SwqosTransport::Quic;
                if use_quic {
                    let quic_endpoint = url.unwrap_or_else(|| {
                        SWQOS_ENDPOINTS_LUNARLANDER_QUIC[region as usize].to_string()
                    });
                    let lunarlander_client = LunarLanderClient::new_quic(
                        rpc_url.clone(),
                        &quic_endpoint,
                        api_key,
                        mev_protection,
                    )
                    .await?;
                    Ok(Arc::new(lunarlander_client))
                } else {
                    let endpoint = SwqosConfig::get_endpoint(SwqosType::LunarLander, region, url);
                    let lunarlander_client =
                        LunarLanderClient::new(rpc_url.clone(), endpoint, api_key);
                    Ok(Arc::new(lunarlander_client))
                }
            }
            SwqosConfig::Glaive(api_key, region, url, transport, _) => {
                match transport.unwrap_or(SwqosTransport::Quic) {
                    SwqosTransport::Quic => {
                        let endpoint = url.unwrap_or_else(|| {
                            SWQOS_ENDPOINTS_GLAIVE_QUIC[region as usize].to_string()
                        });
                        let client = GlaiveClient::new_quic(
                            rpc_url.clone(),
                            &endpoint,
                            api_key,
                            mev_protection,
                        )
                        .await?;
                        Ok(Arc::new(client))
                    }
                    SwqosTransport::Http => {
                        let endpoint = url
                            .unwrap_or_else(|| SWQOS_ENDPOINTS_GLAIVE[region as usize].to_string());
                        let client = GlaiveClient::new_http(
                            rpc_url.clone(),
                            endpoint,
                            api_key,
                            mev_protection,
                        )?;
                        Ok(Arc::new(client))
                    }
                    SwqosTransport::Grpc => {
                        anyhow::bail!("Glaive does not support the gRPC transport")
                    }
                }
            }
            SwqosConfig::Default(endpoint) => {
                let rpc = SolanaRpcClient::new_with_commitment(endpoint, commitment);
                let rpc_client = SolRpcClient::new(Arc::new(rpc));
                Ok(Arc::new(rpc_client))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn decimal_min_tip_applies_to_custom_http_transport_paths() {
        let configs = [
            SwqosConfig::Temporal(
                String::new(),
                SwqosRegion::Default,
                Some("https://temporal.example".into()),
                Some(0.0001),
            ),
            SwqosConfig::BlockRazor(
                String::new(),
                SwqosRegion::Default,
                Some("https://blockrazor.example".into()),
                None,
                Some(0.0001),
            ),
            SwqosConfig::Astralane(
                String::new(),
                SwqosRegion::Default,
                Some("https://astralane.example".into()),
                None,
                Some(0.0001),
            ),
        ];
        for config in configs {
            let client = SwqosConfig::get_swqos_client(
                String::new(),
                CommitmentConfig::confirmed(),
                config,
                false,
            )
            .await
            .unwrap();
            assert_eq!(client.configured_min_tip_lamports(), Some(100_000));
        }
    }

    #[test]
    fn decimal_min_tip_conversion_and_validation() {
        for (minimum, expected) in [
            (None, None),
            (Some(0.0), Some(0)),
            (Some(0.0001), Some(100_000)),
            (Some(0.1), Some(100_000_000)),
            (Some(0.07), Some(70_000_000)),
        ] {
            let config = SwqosConfig::Jito(String::new(), SwqosRegion::Default, None, minimum);
            assert_eq!(config.min_tip_lamports().unwrap(), expected);
        }
        for minimum in [-0.0001, f64::NAN, f64::INFINITY, f64::MAX] {
            let config =
                SwqosConfig::Jito(String::new(), SwqosRegion::Default, None, Some(minimum));
            assert!(config.min_tip_lamports().is_err());
        }
    }

    #[test]
    fn decimal_min_tip_participates_in_config_identity() {
        use std::collections::HashSet;
        let config =
            |minimum| SwqosConfig::Jito(String::new(), SwqosRegion::Default, None, minimum);
        let mut set = HashSet::new();
        assert!(set.insert(config(None)));
        assert!(set.insert(config(Some(0.0001))));
        assert!(!set.insert(config(Some(0.0001))));
        assert!(set.insert(config(Some(0.1))));
        assert_eq!(config(Some(f64::NAN)), config(Some(f64::NAN)));
    }

    #[tokio::test]
    async fn optional_decimal_min_tip_preserves_provider() {
        let config =
            SwqosConfig::Helius(String::new(), SwqosRegion::Default, None, Some(true), None);
        let plain = SwqosConfig::get_swqos_client(
            String::new(),
            CommitmentConfig::confirmed(),
            config.clone(),
            false,
        )
        .await
        .unwrap();
        assert_eq!(plain.configured_min_tip_lamports(), None);
        let configured =
            SwqosConfig::Helius(String::new(), SwqosRegion::Default, None, Some(true), Some(0.1));
        assert_eq!(configured.swqos_type(), SwqosType::Helius);
        let client = SwqosConfig::get_swqos_client(
            String::new(),
            CommitmentConfig::confirmed(),
            configured,
            false,
        )
        .await
        .unwrap();
        assert_eq!(client.configured_min_tip_lamports(), Some(100_000_000));
        assert_eq!(client.min_tip_sol(), plain.min_tip_sol());
        assert!(client.get_tip_account().unwrap().parse::<solana_sdk::pubkey::Pubkey>().is_ok());
    }

    #[test]
    fn lunarlander_defaults_to_quic_endpoint() {
        let endpoint = SwqosConfig::get_endpoint_with_transport(
            SwqosType::LunarLander,
            SwqosRegion::Frankfurt,
            None,
            None,
            false,
        );

        assert_eq!(endpoint, SWQOS_ENDPOINTS_LUNARLANDER_QUIC[SwqosRegion::Frankfurt as usize]);
    }

    #[test]
    fn lunarlander_http_transport_uses_binary_http_endpoint() {
        let endpoint = SwqosConfig::get_endpoint_with_transport(
            SwqosType::LunarLander,
            SwqosRegion::Frankfurt,
            None,
            Some(SwqosTransport::Http),
            false,
        );

        assert_eq!(endpoint, SWQOS_ENDPOINTS_LUNARLANDER[SwqosRegion::Frankfurt as usize]);
    }

    #[test]
    fn glaive_defaults_to_quic_endpoint() {
        assert!(SwqosType::values().contains(&SwqosType::Glaive));
        let endpoint = SwqosConfig::get_endpoint_with_transport(
            SwqosType::Glaive,
            SwqosRegion::Frankfurt,
            None,
            None,
            false,
        );
        assert_eq!(endpoint, SWQOS_ENDPOINTS_GLAIVE_QUIC[SwqosRegion::Frankfurt as usize]);
    }

    #[test]
    fn glaive_http_transport_uses_binary_http_origin() {
        let endpoint = SwqosConfig::get_endpoint_with_transport(
            SwqosType::Glaive,
            SwqosRegion::Frankfurt,
            None,
            Some(SwqosTransport::Http),
            false,
        );
        assert_eq!(endpoint, SWQOS_ENDPOINTS_GLAIVE[SwqosRegion::Frankfurt as usize]);
    }

    #[tokio::test]
    async fn glaive_rejects_unsupported_grpc_transport_without_connecting() {
        let result = SwqosConfig::get_swqos_client(
            "http://127.0.0.1:8899".to_string(),
            CommitmentConfig::processed(),
            SwqosConfig::Glaive(
                "00112233-4455-4677-8899-aabbccddeeff".to_string(),
                SwqosRegion::Frankfurt,
                None,
                Some(SwqosTransport::Grpc),
                None,
            ),
            false,
        )
        .await;
        let error = result.err().expect("Glaive gRPC config must fail");
        assert!(error.to_string().contains("does not support the gRPC transport"));
    }
}
