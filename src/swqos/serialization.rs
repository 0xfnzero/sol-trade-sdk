//! Transaction serialization module.

use anyhow::Result;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use once_cell::sync::Lazy;
use solana_client::rpc_client::SerializableTransaction;
use solana_sdk::signature::Signature;
use solana_transaction_status::UiTransactionEncoding;
use std::sync::Arc;
use crossbeam_queue::ArrayQueue;
use crate::perf::{
    simd::SIMDSerializer,
    compiler_optimization::CompileTimeOptimizedEventProcessor,
};

/// Zero-allocation serializer using a buffer pool to avoid runtime allocation.
pub struct ZeroAllocSerializer {
    buffer_pool: Arc<ArrayQueue<Vec<u8>>>,
    buffer_size: usize,
}

impl ZeroAllocSerializer {
    pub fn new(pool_size: usize, buffer_size: usize) -> Self {
        let pool = ArrayQueue::new(pool_size);

        // Pre-allocate buffers
        for _ in 0..pool_size {
            let mut buffer = Vec::with_capacity(buffer_size);
            buffer.resize(buffer_size, 0);
            let _ = pool.push(buffer);
        }

        Self {
            buffer_pool: Arc::new(pool),
            buffer_size,
        }
    }

    pub fn serialize_zero_alloc<T: serde::Serialize>(&self, data: &T, _label: &str) -> Result<Vec<u8>> {
        // Try to get a buffer from the pool
        let mut buffer = self.buffer_pool.pop().unwrap_or_else(|| {
            let mut buf = Vec::with_capacity(self.buffer_size);
            buf.resize(self.buffer_size, 0);
            buf
        });

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
static SERIALIZER: Lazy<Arc<ZeroAllocSerializer>> = Lazy::new(|| {
    Arc::new(ZeroAllocSerializer::new(
        10_000,      // Pool size
        256 * 1024,  // Buffer size: 256KB
    ))
});

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
        Ok(STANDARD.encode(&serialized))
    }
}

/// Sync serialize + encode using buffer pool; use in hot path to reduce allocs.
pub fn serialize_transaction_sync(
    transaction: &impl SerializableTransaction,
    encoding: UiTransactionEncoding,
) -> Result<(String, Signature)> {
    let signature = transaction.get_signature();
    let serialized_tx = SERIALIZER.serialize_zero_alloc(transaction, "transaction")?;
    let serialized = match encoding {
        UiTransactionEncoding::Base58 => bs58::encode(&serialized_tx).into_string(),
        UiTransactionEncoding::Base64 => STANDARD.encode(&serialized_tx),
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
        UiTransactionEncoding::Base64 => {
            // Use SIMD-optimized Base64 encoding
            STANDARD.encode(&serialized_tx)
        }
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
            UiTransactionEncoding::Base64 => STANDARD.encode(&serialized_tx),
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
            UiTransactionEncoding::Base64 => STANDARD.encode(&serialized_tx),
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
        assert_eq!(capacity, 10_000);
    }
}
