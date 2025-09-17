use crate::common::PriorityFee;
use dashmap::DashMap;
use once_cell::sync::Lazy;
use smallvec::SmallVec;
use solana_sdk::{compute_budget::ComputeBudgetInstruction, instruction::Instruction};

/// Cache key containing all parameters for compute budget instructions
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ComputeBudgetCacheKey {
    data_size_limit: u32,
    unit_price: u64,
    unit_limit: u32,
    is_buy: bool,
}

/// Global cache storing compute budget instructions
/// Uses DashMap for high-performance lock-free concurrent access
static COMPUTE_BUDGET_CACHE: Lazy<DashMap<ComputeBudgetCacheKey, SmallVec<[Instruction; 3]>>> =
    Lazy::new(|| DashMap::new());

#[inline(always)]
pub fn compute_budget_instructions(
    priority_fee: &PriorityFee,
    data_size_limit: u32,
    is_rpc: bool,
    is_buy: bool,
) -> SmallVec<[Instruction; 3]> {
    let (unit_price, unit_limit) = if is_rpc {
        (priority_fee.rpc_unit_price, priority_fee.rpc_unit_limit)
    } else {
        (priority_fee.tip_unit_price, priority_fee.tip_unit_limit)
    };

    // Create cache key
    let cache_key = ComputeBudgetCacheKey { data_size_limit, unit_price, unit_limit, is_buy };

    // Try to get from cache first
    if let Some(cached_insts) = COMPUTE_BUDGET_CACHE.get(&cache_key) {
        return cached_insts.clone();
    }

    // Cache miss, generate new instructions
    let mut insts = SmallVec::<[Instruction; 3]>::new();

    if is_buy {
        insts.push(ComputeBudgetInstruction::set_loaded_accounts_data_size_limit(data_size_limit));
    }

    insts.extend([
        ComputeBudgetInstruction::set_compute_unit_price(unit_price),
        ComputeBudgetInstruction::set_compute_unit_limit(unit_limit),
    ]);

    // Store result in cache
    let insts_clone = insts.clone();
    COMPUTE_BUDGET_CACHE.insert(cache_key, insts_clone);

    insts
}
