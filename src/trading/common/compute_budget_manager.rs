use dashmap::DashMap;
use once_cell::sync::Lazy;
use smallvec::SmallVec;
use solana_sdk::instruction::Instruction;
use solana_compute_budget_interface::ComputeBudgetInstruction;

/// Cache key containing all parameters for compute budget instructions
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ComputeBudgetCacheKey {
    unit_price: u64,
    unit_limit: u32,
}

/// Global cache storing compute budget instructions
/// Uses DashMap for high-performance lock-free concurrent access
static COMPUTE_BUDGET_CACHE: Lazy<DashMap<ComputeBudgetCacheKey, SmallVec<[Instruction; 2]>>> =
    Lazy::new(|| DashMap::new());

#[inline(always)]
pub fn compute_budget_instructions(
    unit_price: u64,
    unit_limit: u32,
) -> SmallVec<[Instruction; 2]> {
    // Create cache key
    let cache_key = ComputeBudgetCacheKey {
        unit_price: unit_price,
        unit_limit: unit_limit,
    };

    // Try to get from cache first
    if let Some(cached_insts) = COMPUTE_BUDGET_CACHE.get(&cache_key) {
        return cached_insts.clone();
    }

    // Cache miss, generate new instructions
    let mut insts = SmallVec::<[Instruction; 2]>::new();

    // Only add compute unit price instruction if > 0
    if unit_price > 0 {
        insts.push(ComputeBudgetInstruction::set_compute_unit_price(unit_price));
    }

    // Only add compute unit limit instruction if > 0
    if unit_limit > 0 {
        insts.push(ComputeBudgetInstruction::set_compute_unit_limit(unit_limit));
    }

    // Store result in cache
    let insts_clone = insts.clone();
    COMPUTE_BUDGET_CACHE.insert(cache_key, insts_clone);

    insts
}
