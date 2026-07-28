//! Event classifier: program ID filter, instruction decode, and event type identification.
//!
//! Runs after slot reconstruction. Filters transactions by allowed program IDs,
//! skips vote transactions, decodes instructions into typed events, and feeds
//! the dedup cache.

use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use solana_sdk::transaction::VersionedTransaction;
use std::collections::HashSet;
use std::sync::Arc;

use crate::perf::shredstream::dedup::{DedupCache, DedupKey};
use crate::perf::shredstream::metrics::ShredstreamMetrics;

/// The type of event encoded from a protocol transaction instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum EventType {
    Swap = 0,
    CreatePool = 1,
    AddLiquidity = 2,
    RemoveLiquidity = 3,
    Unknown = 255,
}

impl From<u8> for EventType {
    fn from(v: u8) -> Self {
        match v {
            0 => EventType::Swap,
            1 => EventType::CreatePool,
            2 => EventType::AddLiquidity,
            3 => EventType::RemoveLiquidity,
            _ => EventType::Unknown,
        }
    }
}

/// A classified event after program filtering, instruction decode, and dedup.
#[derive(Debug, Clone)]
pub struct ClassifiedEvent {
    pub slot: u64,
    pub signature: Signature,
    pub event_type: EventType,
    pub instruction_index: u8,
    pub program_id: Pubkey,
    pub received_at_micros: i64,
    pub decoded_at_micros: i64,
    pub raw_instruction_data: Vec<u8>,
}

/// The vote program ID (constant).
const VOTE_PROGRAM_ID: &str = "Vote111111111111111111111111111111111111111";

/// Classifier that filters transactions and produces classified events.
pub struct EventClassifier {
    program_allowlist: HashSet<Pubkey>,
    filter_votes: bool,
    dedup_cache: DedupCache,
    metrics: Arc<ShredstreamMetrics>,
}

impl EventClassifier {
    /// Create a new classifier with the given program allowlist, vote filtering, and dedup cache.
    pub fn new(
        allowed_programs: &[String],
        filter_votes: bool,
        dedup_cache: DedupCache,
        metrics: Arc<ShredstreamMetrics>,
    ) -> Self {
        let program_allowlist: HashSet<Pubkey> = allowed_programs
            .iter()
            .filter_map(|s| s.parse::<Pubkey>().ok())
            .collect();

        Self {
            program_allowlist,
            filter_votes,
            dedup_cache,
            metrics,
        }
    }

    /// Process a slot's worth of transactions, returning classified events that pass all filters.
    ///
    /// This is the main entry point called from the reconstruction thread.
    /// `received_at_micros` is the stage-1 timestamp from the receive thread.
    pub fn process_slot(
        &mut self,
        slot: u64,
        transactions: &[VersionedTransaction],
        received_at_micros: i64,
    ) -> Vec<ClassifiedEvent> {
        let _now = crate::common::fast_timing::fast_now_micros();
        let mut events = Vec::with_capacity(transactions.len().min(64));

        for tx in transactions.iter() {
            // Skip vote transactions immediately
            if self.filter_votes && self.is_vote_transaction(tx) {
                self.metrics.filter_vote.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                continue;
            }

            let signature = tx.signatures.first().cloned().unwrap_or_default();
            let message = &tx.message;

            // Program ID filter: check if any account key matches our allowlist
            let has_matching_program = message.static_account_keys().iter().any(|pk| {
                self.program_allowlist.contains(pk)
            });

            if !has_matching_program {
                self.metrics.filter_program_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                continue;
            }

            // Decode instructions
            for (ix_index, ix) in message.instructions().iter().enumerate() {
                let program_id = &message.static_account_keys()
                    [ix.program_id_index as usize];

                // Check if this specific instruction's program is in the allowlist
                if !self.program_allowlist.contains(program_id) {
                    continue;
                }

                // Determine event type from instruction data (first 8 bytes = discriminator)
                let event_type = classify_by_discriminator(
                    program_id,
                    &ix.data,
                );

                // Dedup check
                let dedup_key = DedupKey {
                    slot,
                    signature: signature.into(),
                    event_type: event_type as u8,
                    instruction_index: ix_index as u8,
                };

                if !self.dedup_cache.check_and_insert(dedup_key) {
                    self.metrics.dedup_hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    continue;
                }
                self.metrics.dedup_misses.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                let decoded_at = crate::common::fast_timing::fast_now_micros();

                events.push(ClassifiedEvent {
                    slot,
                    signature,
                    event_type,
                    instruction_index: ix_index as u8,
                    program_id: *program_id,
                    received_at_micros,
                    decoded_at_micros: decoded_at as i64,
                    raw_instruction_data: ix.data.clone(),
                });

                self.metrics.events_classified.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }

        events
    }

    /// Returns `true` if a transaction targets the vote program.
    fn is_vote_transaction(&self, tx: &VersionedTransaction) -> bool {
        tx.message.static_account_keys().iter().any(|pk| {
            pk.to_string() == VOTE_PROGRAM_ID
        })
    }

    /// Return a reference to the dedup cache (for stats/exposure).
    pub fn dedup_cache(&self) -> &DedupCache {
        &self.dedup_cache
    }

    /// Return a mutable reference to the dedup cache (for clearing).
    pub fn dedup_cache_mut(&mut self) -> &mut DedupCache {
        &mut self.dedup_cache
    }
}

/// Classify the event type by examining the instruction program ID and data discriminators.
///
/// Known Anchor discriminators (first 8 bytes):
/// - Swap instructions: varies by protocol
fn classify_by_discriminator(_program_id: &Pubkey, data: &[u8]) -> EventType {
    if data.is_empty() {
        return EventType::Unknown;
    }

    // Raydium AMM V4: first byte [9] = swap_base_in, [11] = swap_base_out
    // PumpFun: Anchor SHA256("global:buy")[..8], SHA256("global:sell")[..8]
    // PumpSwap: Anchor-style buy/sell discriminators
    // Meteora: Anchor SHA256("global:swap")[..8]

    // For now, if data has recognizable anchor discriminators for swap
    // we classify as Swap. This is a best-effort heuristic.
    // Full IDL-based decoding will be added per-protocol in later phases.

    // Common swap discriminator prefixes across DEX protocols
    if !data.is_empty() {
        let first_byte = data[0];
        // Raydium AMM V4: swap_base_in = [9], swap_base_out = [11]
        if first_byte == 9 || first_byte == 11 {
            return EventType::Swap;
        }
    }

    // Check known Anchor swap discriminators
    if data.len() >= 8 {
        let disc: [u8; 8] = match data[..8].try_into() {
            Ok(d) => d,
            Err(_) => return EventType::Unknown,
        };

        // Anchor SHA256("global:swap") = [248, 198, 158, 145, 225, 117, 135, 200]
        if disc == [248, 198, 158, 145, 225, 117, 135, 200] {
            return EventType::Swap;
        }
        // Anchor SHA256("global:buy") from PumpFun
        if disc == [102, 6, 61, 18, 1, 218, 235, 234] {
            return EventType::Swap;
        }
        // Anchor SHA256("global:sell") from PumpFun
        if disc == [51, 230, 181, 142, 76, 52, 84, 155] {
            return EventType::Swap;
        }
    }

    EventType::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vote_program_detection() {
        // Vote program ID string
        assert_eq!(VOTE_PROGRAM_ID, "Vote111111111111111111111111111111111111111");
    }

    #[test]
    fn test_event_type_from_u8() {
        assert_eq!(EventType::from(0), EventType::Swap);
        assert_eq!(EventType::from(1), EventType::CreatePool);
        assert_eq!(EventType::from(255), EventType::Unknown);
    }

    #[test]
    fn test_classify_raydium_discriminators() {
        let raydium_v4: Pubkey = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8".parse().unwrap();
        // swap_base_in = [9]
        assert_eq!(classify_by_discriminator(&raydium_v4, &[9]), EventType::Swap);
        // swap_base_out = [11]
        assert_eq!(classify_by_discriminator(&raydium_v4, &[11]), EventType::Swap);
        // unknown instruction = [7]
        assert_eq!(classify_by_discriminator(&raydium_v4, &[7]), EventType::Unknown);
    }

    #[test]
    fn test_classify_empty_data() {
        let pk: Pubkey = Pubkey::new_from_array([0u8; 32]);
        assert_eq!(classify_by_discriminator(&pk, &[]), EventType::Unknown);
    }

    #[test]
    fn test_new_classifier_empty_allowlist() {
        let metrics = ShredstreamMetrics::new_arc();
        let dedup = DedupCache::new(std::num::NonZeroUsize::new(100).unwrap());
        let classifier = EventClassifier::new(&[], false, dedup, metrics);
        assert!(classifier.program_allowlist.is_empty());
    }
}