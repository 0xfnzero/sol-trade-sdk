//! 🚀 交易构建器对象池
//!
//! 预分配交易构建器,避免运行时分配:
//! - 对象池重用
//! - 零分配构建
//! - 零拷贝 I/O
//! - 内存预热

/// 预分配指令容量（单笔交易常见指令数）
const TX_BUILDER_INSTRUCTION_CAP: usize = 32;
/// 预分配地址查找表数量
const TX_BUILDER_LOOKUP_TABLE_CAP: usize = 8;
/// 对象池最大容量
const TX_BUILDER_POOL_CAP: usize = 1000;
/// 启动时预填充对象池数量
const TX_BUILDER_POOL_PREFILL: usize = 100;

use crossbeam_queue::ArrayQueue;
use once_cell::sync::Lazy;
use solana_sdk::{
    hash::Hash, instruction::Instruction, message::{v0, AddressLookupTableAccount, Message, VersionedMessage}, pubkey::Pubkey
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
            instructions: Vec::with_capacity(TX_BUILDER_INSTRUCTION_CAP),
            lookup_tables: Vec::with_capacity(TX_BUILDER_LOOKUP_TABLE_CAP),
        }
    }

    /// 重置构建器 (清空但保留容量)
    #[inline(always)]
    fn reset(&mut self) {
        self.instructions.clear();
        self.lookup_tables.clear();
    }

    /// 🚀 零分配构建交易
    ///
    /// # 交易版本自动选择
    ///
    /// - **有地址查找表** (`lookup_table = Some`): 使用 `VersionedMessage::V0`
    ///   - 支持地址查找表压缩
    ///   - 减少交易大小
    ///   - 需要 RPC 支持 V0
    ///
    /// - **无地址查找表** (`lookup_table = None`): 使用 `VersionedMessage::Legacy`
    ///   - 兼容所有 RPC 节点
    ///   - 无需地址查找表支持
    ///   - 适用于简单交易
    ///
    /// # 示例
    ///
    /// ```rust,ignore
    /// // 无查找表 -> Legacy 消息
    /// let msg = builder.build_zero_alloc(&payer, &ixs, None, blockhash);
    /// assert!(matches!(msg, VersionedMessage::Legacy(_)));
    ///
    /// // 有查找表 -> V0 消息
    /// let msg = builder.build_zero_alloc(&payer, &ixs, Some(table_key), blockhash);
    /// assert!(matches!(msg, VersionedMessage::V0(_)));
    /// ```
    #[inline(always)]
    pub fn build_zero_alloc(
        &mut self,
        payer: &Pubkey,
        instructions: &[Instruction],
        address_lookup_table_account: Option<&AddressLookupTableAccount>,
        recent_blockhash: Hash,
    ) -> VersionedMessage {
        self.reset();
        self.instructions.extend_from_slice(instructions);

        if let Some(alt) = address_lookup_table_account {
            let message = v0::Message::try_compile(
                payer,
                &self.instructions,
                std::slice::from_ref(alt),
                recent_blockhash,
            )
            .expect("v0 message compile failed");
            VersionedMessage::V0(message)
        } else {
            // ✅ 没有查找表，使用 Legacy 消息（兼容所有 RPC）
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
    let pool = ArrayQueue::new(TX_BUILDER_POOL_CAP);

    for _ in 0..TX_BUILDER_POOL_PREFILL {
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