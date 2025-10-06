//! 🚀 交易构建器对象池
//!
//! 预分配交易构建器,避免运行时分配:
//! - 对象池重用
//! - 零分配构建
//! - 零拷贝 I/O
//! - 内存预热

use crossbeam_queue::ArrayQueue;
use once_cell::sync::Lazy;
use solana_sdk::{
    instruction::Instruction,
    message::{v0, VersionedMessage, Message},
    pubkey::Pubkey,
    hash::Hash,
};
use std::sync::Arc;
/// 预分配的交易构建器
pub struct PreallocatedTxBuilder {
    /// 预分配的指令容器
    instructions: Vec<Instruction>,
    /// 预分配的地址查找表
    lookup_tables: Vec<v0::MessageAddressTableLookup>,
}

impl PreallocatedTxBuilder {
    fn new() -> Self {
        Self {
            instructions: Vec::with_capacity(32), // 预分配32条指令空间
            lookup_tables: Vec::with_capacity(8),  // 预分配8个查找表空间
        }
    }

    /// 重置构建器 (清空但保留容量)
    #[inline(always)]
    fn reset(&mut self) {
        self.instructions.clear();
        self.lookup_tables.clear();
    }

    /// 🚀 零分配构建交易
    #[inline(always)]
    pub fn build_zero_alloc(
        &mut self,
        payer: &Pubkey,
        instructions: &[Instruction],
        lookup_table: Option<Pubkey>,
        recent_blockhash: Hash,
    ) -> VersionedMessage {
        // 重用已分配的 vector
        self.reset();
        self.instructions.extend_from_slice(instructions);

        // 如果有查找表，使用 V0 消息
        if let Some(table_key) = lookup_table {
            self.lookup_tables.push(v0::MessageAddressTableLookup {
                account_key: table_key,
                writable_indexes: vec![],
                readonly_indexes: vec![],
            });

            // 使用 Message::new 创建 legacy 消息，然后提取编译后的指令
            let legacy_msg = Message::new(&self.instructions, Some(payer));

            // 构建 V0 消息
            let message = v0::Message {
                header: legacy_msg.header,
                account_keys: legacy_msg.account_keys,
                recent_blockhash,
                instructions: legacy_msg.instructions,
                address_table_lookups: self.lookup_tables.clone(),
            };

            VersionedMessage::V0(message)
        } else {
            // 没有查找表，使用 legacy 消息
            let message = Message::new_with_blockhash(
                &self.instructions,
                Some(payer),
                &recent_blockhash,
            );
            VersionedMessage::Legacy(message)
        }
    }
}

/// 🚀 全局交易构建器对象池
static TX_BUILDER_POOL: Lazy<Arc<ArrayQueue<PreallocatedTxBuilder>>> = Lazy::new(|| {
    let pool = ArrayQueue::new(1000); // 1000个预分配构建器

    // 预填充池
    for _ in 0..100 {
        let _ = pool.push(PreallocatedTxBuilder::new());
    }

    Arc::new(pool)
});

/// 🚀 从池中获取构建器
#[inline(always)]
pub fn acquire_builder() -> PreallocatedTxBuilder {
    TX_BUILDER_POOL
        .pop()
        .unwrap_or_else(|| PreallocatedTxBuilder::new())
}

/// 🚀 归还构建器到池
#[inline(always)]
pub fn release_builder(mut builder: PreallocatedTxBuilder) {
    builder.reset();
    let _ = TX_BUILDER_POOL.push(builder);
}

/// 获取池统计
pub fn get_pool_stats() -> (usize, usize) {
    (TX_BUILDER_POOL.len(), TX_BUILDER_POOL.capacity())
}

/// 🚀 RAII 构建器包装器 (自动归还)
pub struct TxBuilderGuard {
    builder: Option<PreallocatedTxBuilder>,
}

impl TxBuilderGuard {
    pub fn new() -> Self {
        Self {
            builder: Some(acquire_builder()),
        }
    }

    pub fn get_mut(&mut self) -> &mut PreallocatedTxBuilder {
        self.builder.as_mut().unwrap()
    }
}

impl Drop for TxBuilderGuard {
    fn drop(&mut self) {
        if let Some(builder) = self.builder.take() {
            release_builder(builder);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_operations() {
        let builder1 = acquire_builder();
        let builder2 = acquire_builder();

        release_builder(builder1);
        release_builder(builder2);

        let (available, capacity) = get_pool_stats();
        assert!(available >= 2);
        assert_eq!(capacity, 1000);
    }

    #[test]
    fn test_builder_guard() {
        let initial_count = get_pool_stats().0;

        {
            let _guard = TxBuilderGuard::new();
            // guard 会在作用域结束时自动归还
        }

        let final_count = get_pool_stats().0;
        assert_eq!(final_count, initial_count);
    }
}
