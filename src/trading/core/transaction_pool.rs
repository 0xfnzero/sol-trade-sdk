//! Transaction message compilation from borrowed instructions.
//!
//! The compatibility builder/pool API remains available to existing callers.
//! Sender hot paths use `compile_message` directly; official message compilation
//! allocates its result, without an intermediate Instruction clone.

/// 对象池最大容量
const TX_BUILDER_POOL_CAP: usize = 1000;
/// Compatibility pool prefill lower bound.
const PARALLEL_SENDER_COUNT: usize = 18;
/// Compatibility handles prefilled at initialization; senders bypass this pool.
const TX_BUILDER_POOL_PREFILL: usize = 64;

use crate::common::TradeTransactionVersion;
use anyhow::Result;
use crossbeam_queue::ArrayQueue;
use once_cell::sync::Lazy;
use solana_message::AddressLookupTableAccount;
use solana_sdk::{
    hash::Hash,
    instruction::Instruction,
    message::{v0, v1, Message, VersionedMessage},
    pubkey::Pubkey,
};
use std::sync::Arc;
/// Compatibility handle; compilation has no per-handle scratch state.
pub struct PreallocatedTxBuilder;

impl PreallocatedTxBuilder {
    fn new() -> Self {
        Self
    }

    /// Compile directly from the supplied slice. The method name is retained
    /// for compatibility; the official compiler allocates message storage.
    ///
    /// # 交易版本选择
    ///
    /// - `V0` 无地址查找表时构造 Legacy，有地址查找表时构造 V0，保持原有兼容行为。
    /// - `V1` 构造 `VersionedMessage::V1`，并拒绝地址查找表。
    ///
    /// # 示例
    ///
    /// ```rust,ignore
    /// let msg = builder.build_zero_alloc(
    ///     &payer,
    ///     &ixs,
    ///     &[lookup_table],
    ///     blockhash,
    ///     TradeTransactionVersion::V0,
    ///     v1::TransactionConfig::empty(),
    /// );
    /// assert!(matches!(msg, VersionedMessage::V0(_)));
    /// ```
    #[inline(always)]
    pub fn build_zero_alloc(
        &mut self,
        payer: &Pubkey,
        instructions: &[Instruction],
        address_lookup_table_accounts: &[AddressLookupTableAccount],
        recent_blockhash: Hash,
        transaction_version: TradeTransactionVersion,
        v1_config: v1::TransactionConfig,
    ) -> Result<VersionedMessage> {
        compile_message(
            payer,
            instructions,
            address_lookup_table_accounts,
            recent_blockhash,
            transaction_version,
            v1_config,
        )
    }
}

/// Compile a message without copying the input instructions or acquiring a pool.
#[inline]
pub fn compile_message(
    payer: &Pubkey,
    instructions: &[Instruction],
    address_lookup_table_accounts: &[AddressLookupTableAccount],
    recent_blockhash: Hash,
    transaction_version: TradeTransactionVersion,
    v1_config: v1::TransactionConfig,
) -> Result<VersionedMessage> {
    match transaction_version {
        TradeTransactionVersion::V0 => {
            if address_lookup_table_accounts.is_empty() {
                let message =
                    Message::new_with_blockhash(instructions, Some(payer), &recent_blockhash);
                Ok(VersionedMessage::Legacy(message))
            } else {
                let message = v0::Message::try_compile(
                    payer,
                    instructions,
                    address_lookup_table_accounts,
                    recent_blockhash,
                )?;
                Ok(VersionedMessage::V0(message))
            }
        }
        TradeTransactionVersion::V1 => {
            if !address_lookup_table_accounts.is_empty() {
                anyhow::bail!("V1 transactions do not support address lookup tables");
            }
            let message = v1::Message::try_compile_with_config(
                payer,
                instructions,
                recent_blockhash,
                v1_config,
            )?;
            Ok(VersionedMessage::V1(message))
        }
    }
}

/// 🚀 全局交易构建器对象池
static TX_BUILDER_POOL: Lazy<Arc<ArrayQueue<PreallocatedTxBuilder>>> = Lazy::new(|| {
    let pool = ArrayQueue::new(TX_BUILDER_POOL_CAP);
    let prefill = TX_BUILDER_POOL_PREFILL.max(PARALLEL_SENDER_COUNT);
    for _ in 0..prefill {
        let _ = pool.push(PreallocatedTxBuilder::new());
    }
    Arc::new(pool)
});

/// 🚀 从池中获取构建器
#[inline(always)]
pub fn acquire_builder() -> PreallocatedTxBuilder {
    TX_BUILDER_POOL.pop().unwrap_or_else(PreallocatedTxBuilder::new)
}

/// 🚀 归还构建器到池
#[inline(always)]
pub fn release_builder(builder: PreallocatedTxBuilder) {
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
        Self { builder: Some(acquire_builder()) }
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
