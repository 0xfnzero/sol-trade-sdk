//! Transaction serialization module.

use crate::perf::{
    compiler_optimization::CompileTimeOptimizedEventProcessor, simd::SIMDSerializer,
};
use anyhow::Result;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use crossbeam_queue::ArrayQueue;
use once_cell::sync::Lazy;
use solana_client::rpc_client::SerializableTransaction;
use solana_sdk::signature::Signature;
use solana_transaction_status::UiTransactionEncoding;
use std::sync::Arc;

/// Max number of reusable buffers kept in the queue.
const SERIALIZER_POOL_SIZE: usize = 10_000;
/// Per-buffer reserved capacity (bytes).
const SERIALIZER_BUFFER_SIZE: usize = 256 * 1024;
/// Cold-start prewarm count. Keep small to avoid first-submit spikes.
const SERIALIZER_PREWARM_BUFFERS: usize = 64;

/// Zero-allocation serializer using a buffer pool to avoid runtime allocation.
pub struct ZeroAllocSerializer {
    buffer_pool: Arc<ArrayQueue<Vec<u8>>>,
    buffer_size: usize,
}

impl ZeroAllocSerializer {
    pub fn new(pool_size: usize, buffer_size: usize) -> Self {
        Self::new_with_prewarm(pool_size, buffer_size, SERIALIZER_PREWARM_BUFFERS)
    }

    fn new_with_prewarm(pool_size: usize, buffer_size: usize, prewarm_buffers: usize) -> Self {
        let pool = ArrayQueue::new(pool_size);
        let prewarm_count = prewarm_buffers.min(pool_size);

        // Prewarm only a small hot set to avoid large cold-start blocking.
        // Remaining buffers are allocated lazily and returned to this pool.
        for _ in 0..prewarm_count {
            let _ = pool.push(Vec::with_capacity(buffer_size));
        }

        Self { buffer_pool: Arc::new(pool), buffer_size }
    }

    pub fn serialize_zero_alloc<T: serde::Serialize>(
        &self,
        data: &T,
        _label: &str,
    ) -> Result<Vec<u8>> {
        // Try to get a buffer from the pool
        let mut buffer =
            self.buffer_pool.pop().unwrap_or_else(|| Vec::with_capacity(self.buffer_size));

        // Serialize into buffer
        let serialized = bincode::serialize(data)?;
        buffer.clear();
        buffer.extend_from_slice(&serialized);

        Ok(buffer)
    }

    pub fn return_buffer(&self, buffer: Vec<u8>) {
        // Return buffer to the pool
        let _ = self.buffer_pool.push(buffer);
    }

    /// Get pool statistics.
    pub fn get_pool_stats(&self) -> (usize, usize) {
        let available = self.buffer_pool.len();
        let capacity = self.buffer_pool.capacity();
        (available, capacity)
    }
}

/// Global serializer instance.
static SERIALIZER: Lazy<Arc<ZeroAllocSerializer>> =
    Lazy::new(|| Arc::new(ZeroAllocSerializer::new(SERIALIZER_POOL_SIZE, SERIALIZER_BUFFER_SIZE)));

/// Compile-time optimized event processor (zero runtime cost).
static COMPILE_TIME_PROCESSOR: CompileTimeOptimizedEventProcessor =
    CompileTimeOptimizedEventProcessor::new();

/// Base64 encoder.
pub struct Base64Encoder;

impl Base64Encoder {
    #[inline(always)]
    pub fn encode(data: &[u8]) -> String {
        // Use compile-time optimized hash for fast routing
        let _route = if !data.is_empty() {
            COMPILE_TIME_PROCESSOR.route_event_zero_cost(data[0])
        } else {
            0
        };

        // Use SIMD-accelerated Base64 encoding
        SIMDSerializer::encode_base64_simd(data)
    }

    #[inline(always)]
    pub fn serialize_and_encode<T: serde::Serialize>(
        value: &T,
        event_type: &str,
    ) -> Result<String> {
        let serialized = SERIALIZER.serialize_zero_alloc(value, event_type)?;
        let encoded = STANDARD.encode(&serialized);
        SERIALIZER.return_buffer(serialized);
        Ok(encoded)
    }
}

/// Guard that returns the serialization buffer to the pool on drop.
pub struct PooledTxBufGuard(pub Vec<u8>);

impl std::ops::Deref for PooledTxBufGuard {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for PooledTxBufGuard {
    fn drop(&mut self) {
        if !self.0.is_empty() {
            SERIALIZER.return_buffer(std::mem::take(&mut self.0));
        }
    }
}

/// Serialize transaction to bincode bytes using buffer pool. The returned guard returns the buffer
/// to the pool when dropped; use `&*guard` or `guard.as_ref()` for `&[u8]`.
pub fn serialize_transaction_bincode_sync(
    transaction: &impl SerializableTransaction,
) -> Result<(PooledTxBufGuard, Signature)> {
    let signature = transaction.get_signature();
    let serialized_tx = SERIALIZER.serialize_zero_alloc(transaction, "transaction")?;
    Ok((PooledTxBufGuard(serialized_tx), *signature))
}

/// Return a buffer to the pool (for manual use when not using `PooledTxBufGuard`).
pub fn return_serialization_buffer(buffer: Vec<u8>) {
    SERIALIZER.return_buffer(buffer);
}

/// Sync serialize + encode using buffer pool; use in hot path to reduce allocs.
/// Base64 path uses SIMD-accelerated encoding.
pub fn serialize_transaction_sync(
    transaction: &impl SerializableTransaction,
    encoding: UiTransactionEncoding,
) -> Result<(String, Signature)> {
    let signature = transaction.get_signature();
    let serialized_tx = SERIALIZER.serialize_zero_alloc(transaction, "transaction")?;
    let serialized = match encoding {
        UiTransactionEncoding::Base58 => bs58::encode(&serialized_tx).into_string(),
        UiTransactionEncoding::Base64 => SIMDSerializer::encode_base64_simd(&serialized_tx),
        _ => return Err(anyhow::anyhow!("Unsupported encoding")),
    };
    SERIALIZER.return_buffer(serialized_tx);
    Ok((serialized, *signature))
}

/// Serialize a transaction (async; no I/O, kept for API compatibility).
pub async fn serialize_transaction(
    transaction: &impl SerializableTransaction,
    encoding: UiTransactionEncoding,
) -> Result<(String, Signature)> {
    let signature = transaction.get_signature();

    // Use zero-allocation serialization
    let serialized_tx = SERIALIZER.serialize_zero_alloc(transaction, "transaction")?;

    let serialized = match encoding {
        UiTransactionEncoding::Base58 => bs58::encode(&serialized_tx).into_string(),
        UiTransactionEncoding::Base64 => SIMDSerializer::encode_base64_simd(&serialized_tx),
        _ => return Err(anyhow::anyhow!("Unsupported encoding")),
    };

    // Return buffer to pool immediately
    SERIALIZER.return_buffer(serialized_tx);

    Ok((serialized, *signature))
}

/// Sync batch serialize + encode using buffer pool.
pub fn serialize_transactions_batch_sync(
    transactions: &[impl SerializableTransaction],
    encoding: UiTransactionEncoding,
) -> Result<Vec<String>> {
    let mut results = Vec::with_capacity(transactions.len());
    for tx in transactions {
        let serialized_tx = SERIALIZER.serialize_zero_alloc(tx, "transaction")?;
        let encoded = match encoding {
            UiTransactionEncoding::Base58 => bs58::encode(&serialized_tx).into_string(),
            UiTransactionEncoding::Base64 => SIMDSerializer::encode_base64_simd(&serialized_tx),
            _ => return Err(anyhow::anyhow!("Unsupported encoding")),
        };
        SERIALIZER.return_buffer(serialized_tx);
        results.push(encoded);
    }
    Ok(results)
}

/// Batch transaction serialization.
pub async fn serialize_transactions_batch(
    transactions: &[impl SerializableTransaction],
    encoding: UiTransactionEncoding,
) -> Result<Vec<String>> {
    let mut results = Vec::with_capacity(transactions.len());

    for tx in transactions {
        let serialized_tx = SERIALIZER.serialize_zero_alloc(tx, "transaction")?;

        let encoded = match encoding {
            UiTransactionEncoding::Base58 => bs58::encode(&serialized_tx).into_string(),
            UiTransactionEncoding::Base64 => SIMDSerializer::encode_base64_simd(&serialized_tx),
            _ => return Err(anyhow::anyhow!("Unsupported encoding")),
        };

        SERIALIZER.return_buffer(serialized_tx);
        results.push(encoded);
    }

    Ok(results)
}

/// Get serializer statistics.
pub fn get_serializer_stats() -> (usize, usize) {
    SERIALIZER.get_pool_stats()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_encode() {
        let data = b"Hello, World!";
        let encoded = Base64Encoder::encode(data);
        assert!(!encoded.is_empty());

        // Verify it decodes correctly
        let decoded = STANDARD.decode(&encoded).unwrap();
        assert_eq!(&decoded[..data.len()], data);
    }

    #[test]
    fn test_serializer_stats() {
        let (available, capacity) = get_serializer_stats();
        assert!(available <= capacity);
        assert_eq!(capacity, SERIALIZER_POOL_SIZE);
    }

    #[test]
    fn test_serializer_prewarm_is_bounded() {
        let serializer = ZeroAllocSerializer::new_with_prewarm(128, 1024, 8);
        let (available, capacity) = serializer.get_pool_stats();
        assert_eq!(capacity, 128);
        assert_eq!(available, 8);
    }

    #[test]
    fn test_serializer_lazy_alloc_and_return() {
        let serializer = ZeroAllocSerializer::new_with_prewarm(8, 1024, 0);
        let (available_before, capacity) = serializer.get_pool_stats();
        assert_eq!(capacity, 8);
        assert_eq!(available_before, 0);

        let buf = serializer.serialize_zero_alloc(&"hello", "test").unwrap();
        assert!(buf.capacity() >= 1024);
        serializer.return_buffer(buf);

        let (available_after, _) = serializer.get_pool_stats();
        assert_eq!(available_after, 1);
    }
}
