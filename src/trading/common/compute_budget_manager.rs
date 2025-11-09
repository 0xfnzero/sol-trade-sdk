use dashmap::DashMap;
use once_cell::sync::Lazy;
use smallvec::SmallVec;
use solana_sdk::instruction::Instruction;
use solana_compute_budget_interface::ComputeBudgetInstruction;

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
    unit_price: u64,
    unit_limit: u32,
    data_size_limit: u32,
    is_buy: bool,
) -> SmallVec<[Instruction; 3]> {
    // Create cache key
    let cache_key = ComputeBudgetCacheKey {
        data_size_limit,
        unit_price: unit_price,
        unit_limit: unit_limit,
        is_buy,
    };

    // Try to get from cache first
    if let Some(cached_insts) = COMPUTE_BUDGET_CACHE.get(&cache_key) {
        return cached_insts.clone();
    }

    // Cache miss, generate new instructions
    let mut insts = SmallVec::<[Instruction; 3]>::new();

    // Only add data_size_limit instruction if > 0 and is_buy
    if is_buy && data_size_limit > 0 {
        insts.push(ComputeBudgetInstruction::set_loaded_accounts_data_size_limit(data_size_limit));
    }

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
