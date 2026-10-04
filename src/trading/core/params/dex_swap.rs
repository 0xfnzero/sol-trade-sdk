use crate::common::nonce_cache::DurableNonceInfo;
use crate::common::{GasFeeStrategy, SolanaRpcClient, TradeTransactionVersion};
use crate::swqos::{SwqosClient, TradeType};
use crate::trading::MiddlewareManager;
use crate::PrecheckConfig;
use core_affinity::CoreId;
use solana_hash::Hash;
use solana_message::AddressLookupTableAccount;
use solana_sdk::{pubkey::Pubkey, signature::Keypair};
use std::sync::Arc;

use super::bonk::BonkParams;
use super::meteora_damm_v2::MeteoraDammV2Params;
use super::meteora_dbc::MeteoraDbcParams;
use super::meteora_dlmm::MeteoraDlmmParams;
use super::pumpfun::PumpFunParams;
use super::pumpswap::PumpSwapParams;
use super::raydium_amm_v4::RaydiumAmmV4Params;
use super::raydium_clmm::RaydiumClmmParams;
use super::raydium_cpmm::RaydiumCpmmParams;
use super::stonkfun_via_sol::StonkFunViaSolParams;
use super::whirlpool::WhirlpoolParams;

/// Concurrency + core binding config for parallel submit (precomputed at SDK init, one param on hot path). Uses Arc so no borrow of SwapParams.
#[derive(Clone)]
pub struct SenderConcurrencyConfig {
    pub sender_thread_cores: Option<Arc<Vec<usize>>>,
    pub effective_core_ids: Arc<Vec<CoreId>>,
    pub max_sender_concurrency: usize,
}

/// DEX 参数枚举 - 零开销抽象替代 Box<dyn ProtocolParams>
#[derive(Clone)]
pub enum DexParamEnum {
    PumpFun(PumpFunParams),
    PumpSwap(PumpSwapParams),
    LaunchLab(BonkParams),
    Bonk(BonkParams),
    StonkFun(BonkParams),
    /// Graduated StonkFun pool parameters backed by the external CPMM venue.
    StonkFunSwap(RaydiumCpmmParams),
    /// SOL/WSOL/USDC ↔ quote ↔ meme, or direct quote↔meme when the endpoint matches.
    ///
    /// Works for both the LaunchLab curve and graduated CPMM meme legs.
    StonkFunViaSol(StonkFunViaSolParams),
    /// Standalone quote conversion, also used by StonkFunViaSol internally.
    StonkFunQuoteRoute(super::StonkFunQuoteRoute),
    RaydiumCpmm(RaydiumCpmmParams),
    RaydiumAmmV4(RaydiumAmmV4Params),
    MeteoraDammV2(MeteoraDammV2Params),
    /// Meteora Dynamic Bonding Curve pool, the curve a token trades on before
    /// it migrates to a DAMM pool.
    MeteoraDbc(MeteoraDbcParams),
    RaydiumClmm(RaydiumClmmParams),
    OrcaWhirlpool(WhirlpoolParams),
    MeteoraDlmm(MeteoraDlmmParams),
}

impl DexParamEnum {
    /// 获取内部参数的 Any 引用，用于向后兼容的类型检查
    #[inline]
    pub fn as_any(&self) -> &dyn std::any::Any {
        match self {
            DexParamEnum::PumpFun(p) => p,
            DexParamEnum::PumpSwap(p) => p,
            DexParamEnum::LaunchLab(p) => p,
            DexParamEnum::Bonk(p) => p,
            DexParamEnum::StonkFun(p) => p,
            DexParamEnum::StonkFunSwap(p) => p,
            DexParamEnum::StonkFunViaSol(p) => p,
            DexParamEnum::StonkFunQuoteRoute(p) => p,
            DexParamEnum::RaydiumCpmm(p) => p,
            DexParamEnum::RaydiumAmmV4(p) => p,
            DexParamEnum::MeteoraDammV2(p) => p,
            DexParamEnum::MeteoraDbc(p) => p,
            DexParamEnum::RaydiumClmm(p) => p,
            DexParamEnum::OrcaWhirlpool(p) => p,
            DexParamEnum::MeteoraDlmm(p) => p,
        }
    }
}

/// Swap parameters
#[derive(Clone)]
pub struct SwapParams {
    pub rpc: Option<Arc<SolanaRpcClient>>,
    pub payer: Arc<Keypair>,
    pub trade_type: TradeType,
    pub input_mint: Pubkey,
    pub input_token_program: Option<Pubkey>,
    pub output_mint: Pubkey,
    pub output_token_program: Option<Pubkey>,
    pub input_amount: Option<u64>,
    pub slippage_basis_points: Option<u64>,
    pub address_lookup_table_accounts: Vec<AddressLookupTableAccount>,
    pub recent_blockhash: Option<Hash>,
    pub wait_tx_confirmed: bool,
    pub protocol_params: DexParamEnum,
    pub open_seed_optimize: bool,
    /// Arc<Vec<..>> so cloning from infrastructure is a single Arc clone.
    pub swqos_clients: Arc<Vec<Arc<SwqosClient>>>,
    pub middleware_manager: Option<Arc<MiddlewareManager>>,
    pub durable_nonce: Option<DurableNonceInfo>,
    pub with_tip: bool,
    pub create_input_mint_ata: bool,
    pub close_input_mint_ata: bool,
    pub create_output_mint_ata: bool,
    pub close_output_mint_ata: bool,
    /// Fixed output amount. For protocols with exact-out instructions this selects exact-out
    /// semantics and treats `input_amount` as the maximum input budget.
    pub fixed_output_amount: Option<u64>,
    pub gas_fee_strategy: GasFeeStrategy,
    pub simulate: bool,
    /// Whether to output SDK logs (from TradeConfig.log_enabled).
    pub log_enabled: bool,
    /// When true, wait for every SWQOS route's HTTP submit response before
    /// returning so the result includes all submitted signatures.
    ///
    /// This is useful when confirmation or external monitoring polls all
    /// signatures: each route can submit a distinct transaction because relay
    /// tips may use different accounts. With a durable nonce, at most one route
    /// can consume the nonce; with a recent blockhash, multiple route variants
    /// may be valid, so callers must choose strategy and account state
    /// accordingly. Defaults to false for lower submit latency.
    pub wait_for_all_submits: bool,
    /// Use dedicated sender threads (internal; set via client.with_dedicated_sender_threads()).
    pub use_dedicated_sender_threads: bool,
    /// Core indices for dedicated sender threads (from TradeConfig.sender_thread_cores). Arc avoids cloning the Vec on hot path.
    pub sender_thread_cores: Option<Arc<Vec<usize>>>,
    /// Precomputed at SDK init: min(swqos_count, 2/3*cores). Avoids get_core_ids() on trade hot path.
    pub max_sender_concurrency: usize,
    /// Precomputed at SDK init: first max_sender_concurrency CoreIds for job affinity. Arc clone only.
    pub effective_core_ids: Arc<Vec<CoreId>>,
    /// Whether to check minimum tip per SWQOS (from TradeConfig.check_min_tip). When false, skip filter for lower latency.
    pub check_min_tip: bool,
    /// Solana transaction construction mode selected by `TradeConfig`.
    pub transaction_version: TradeTransactionVersion,
    /// Optional event receive time in microseconds (same scale as sol-parser-sdk clock::now_micros). Used as timing start when log_enabled.
    pub grpc_recv_us: Option<i64>,
    /// Use exact quote-input buy instructions (legacy PumpFun uses SOL quote; V2/PumpSwap use generic quote).
    /// When Some(true) or None (default), the exact SOL/quote amount is spent and slippage is applied to output tokens.
    /// When Some(false), uses regular buy instruction where slippage is applied to SOL/quote input.
    /// This option only applies to PumpFun and PumpSwap DEXes; it is ignored for other DEXes.
    pub use_exact_sol_amount: Option<bool>,
    /// Optional on-chain precheck instruction inserted before PumpFun buy flow.
    pub precheck: Option<PrecheckConfig>,
}

impl SwapParams {
    /// Removes executor RPC access and rejects RPC-dependent simulation/confirmation.
    /// Supply blockhash/nonce and ALTs from background caches. Configure submission
    /// providers separately (an RPC sender still submits over RPC).
    pub fn without_rpc(mut self) -> anyhow::Result<Self> {
        anyhow::ensure!(!self.simulate, "RPC-free execution cannot simulate transactions");
        anyhow::ensure!(
            !self.wait_tx_confirmed,
            "RPC-free execution requires subscription-based confirmation outside the executor"
        );
        self.rpc = None;
        Ok(self)
    }
    /// One struct for execute_parallel: merges sender_thread_cores, effective_core_ids, max_sender_concurrency. Arc clone only.
    #[inline]
    pub fn sender_concurrency_config(&self) -> SenderConcurrencyConfig {
        SenderConcurrencyConfig {
            sender_thread_cores: self.sender_thread_cores.clone(),
            effective_core_ids: self.effective_core_ids.clone(),
            max_sender_concurrency: self.max_sender_concurrency,
        }
    }
}

impl std::fmt::Debug for SwapParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SwapParams: ...")
    }
}
