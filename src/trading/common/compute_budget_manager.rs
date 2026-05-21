use dashmap::DashMap;
use once_cell::sync::Lazy;
use smallvec::SmallVec;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_sdk::instruction::Instruction;
use std::{hash::Hash, sync::Arc};

const MAX_COMPUTE_BUDGET_CACHE_SIZE: usize = 4_096;

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

#[inline]
fn prune_cache<K, V>(cache: &DashMap<K, V>, max_size: usize)
where
    K: Eq + Hash + Clone,
{
    let len = cache.len();
    if len <= max_size {
        return;
    }
    let remove_count = (len - max_size).max(max_size / 16).min(len);
    let keys: Vec<K> = cache.iter().take(remove_count).map(|entry| entry.key().clone()).collect();
    for key in keys {
        cache.remove(&key);
    }
}

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
    prune_cache(&COMPUTE_BUDGET_CACHE, MAX_COMPUTE_BUDGET_CACHE_SIZE);
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
    prune_cache(&COMPUTE_BUDGET_CACHE, MAX_COMPUTE_BUDGET_CACHE_SIZE);
    insts
}
