use dashmap::DashMap;
use once_cell::sync::Lazy;
use smallvec::SmallVec;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_sdk::instruction::Instruction;
use std::sync::Arc;

/// Cache key containing all parameters for compute budget instructions
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ComputeBudgetCacheKey {
    unit_price: u64,
    unit_limit: u32,
}

/// Global cache storing compute budget instructions (Arc to avoid clone on hit).
/// Uses DashMap for high-performance lock-free concurrent access.
static COMPUTE_BUDGET_CACHE: Lazy<DashMap<ComputeBudgetCacheKey, Arc<SmallVec<[Instruction; 2]>>>> =
    Lazy::new(|| DashMap::new());

/// Extend `instructions` with compute budget instructions; on cache hit extends from cached Arc (no SmallVec clone).
#[inline(always)]
pub fn extend_compute_budget_instructions(
    instructions: &mut Vec<Instruction>,
    unit_price: u64,
    unit_limit: u32,
) {
    let cache_key = ComputeBudgetCacheKey { unit_price, unit_limit };

    if let Some(cached) = COMPUTE_BUDGET_CACHE.get(&cache_key) {
        instructions.extend(cached.iter().cloned());
        return;
    }

    let mut insts = SmallVec::<[Instruction; 2]>::new();
    if unit_price > 0 {
        insts.push(ComputeBudgetInstruction::set_compute_unit_price(unit_price));
    }
    if unit_limit > 0 {
        insts.push(ComputeBudgetInstruction::set_compute_unit_limit(unit_limit));
    }
    let arc = Arc::new(insts);
    instructions.extend(arc.iter().cloned());
    COMPUTE_BUDGET_CACHE.insert(cache_key, arc);
}

/// Returns compute budget instructions (allocates on cache hit; prefer `extend_compute_budget_instructions` on hot path).
#[inline(always)]
pub fn compute_budget_instructions(unit_price: u64, unit_limit: u32) -> SmallVec<[Instruction; 2]> {
    let cache_key = ComputeBudgetCacheKey { unit_price, unit_limit };
    if let Some(cached) = COMPUTE_BUDGET_CACHE.get(&cache_key) {
        return (**cached).clone();
    }
    let mut insts = SmallVec::<[Instruction; 2]>::new();
    if unit_price > 0 {
        insts.push(ComputeBudgetInstruction::set_compute_unit_price(unit_price));
    }
    if unit_limit > 0 {
        insts.push(ComputeBudgetInstruction::set_compute_unit_limit(unit_limit));
    }
    let arc = Arc::new(insts.clone());
    COMPUTE_BUDGET_CACHE.insert(cache_key, arc);
    insts
}
