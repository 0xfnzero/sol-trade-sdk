use crate::common::nonce_cache::DurableNonceInfo;
use crate::common::{GasFeeStrategy, SolanaRpcClient};
use crate::swqos::{SwqosClient, TradeType};
use crate::trading::MiddlewareManager;
use core_affinity::CoreId;
use solana_hash::Hash;
use solana_message::AddressLookupTableAccount;
use solana_sdk::{pubkey::Pubkey, signature::Keypair};
use std::sync::Arc;

use super::bonk::BonkParams;
use super::meteora_damm_v2::MeteoraDammV2Params;
use super::pumpfun::PumpFunParams;
use super::pumpswap::PumpSwapParams;
use super::raydium_amm_v4::RaydiumAmmV4Params;
use super::raydium_cpmm::RaydiumCpmmParams;

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
    Bonk(BonkParams),
    RaydiumCpmm(RaydiumCpmmParams),
    RaydiumAmmV4(RaydiumAmmV4Params),
    MeteoraDammV2(MeteoraDammV2Params),
}

impl DexParamEnum {
    /// 获取内部参数的 Any 引用，用于向后兼容的类型检查
    #[inline]
    pub fn as_any(&self) -> &dyn std::any::Any {
        match self {
            DexParamEnum::PumpFun(p) => p,
            DexParamEnum::PumpSwap(p) => p,
            DexParamEnum::Bonk(p) => p,
            DexParamEnum::RaydiumCpmm(p) => p,
            DexParamEnum::RaydiumAmmV4(p) => p,
            DexParamEnum::MeteoraDammV2(p) => p,
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
    pub address_lookup_table_account: Option<AddressLookupTableAccount>,
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
    /// Optional event receive time in microseconds (same scale as sol-parser-sdk clock::now_micros). Used as timing start when log_enabled.
    pub grpc_recv_us: Option<i64>,
    /// Use exact quote-input buy instructions (legacy PumpFun uses SOL quote; V2/PumpSwap use generic quote).
    /// When Some(true) or None (default), the exact SOL/quote amount is spent and slippage is applied to output tokens.
    /// When Some(false), uses regular buy instruction where slippage is applied to SOL/quote input.
    /// This option only applies to PumpFun and PumpSwap DEXes; it is ignored for other DEXes.
    pub use_exact_sol_amount: Option<bool>,
}

impl SwapParams {
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
