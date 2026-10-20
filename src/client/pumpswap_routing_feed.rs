//! Background account ingestion, independent of the transaction-signal transport.
use super::pumpswap_routing::PumpSwapRouting;
use crate::trading::core::params::{CacheReadContext, CachedAccount, SubscriptionAccountCache};
use anyhow::{ensure, Result};
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;

#[derive(Clone, Copy)]
struct ChainClock {
    slot: u64,
    epoch: u64,
    unix_timestamp: u64,
}

/// Background-only cache builder. One feed must represent one provider/fork.
/// Call `reset` on reconnect, event loss, filter replacement or fork rollback.
/// This does not discover accounts or bootstrap missing state over RPC.
#[derive(Default)]
pub struct PumpSwapRoutingFeed {
    cache: SubscriptionAccountCache,
    clock: Option<ChainClock>,
    maximum_seen_slot: u64,
    revision: u64,
    published_revision: Option<u64>,
    continuous_through: Option<u64>,
}
impl PumpSwapRoutingFeed {
    /// Assert that every cached account was bootstrapped at this coherent slot,
    /// and all subsequent updates are delivered without gaps on the same fork.
    /// A last-write slot is not evidence of bootstrap completeness. The caller
    /// must obtain a snapshot plus ordered catch-up before making this assertion.
    /// Reset on reconnect, loss, filter changes or rollback before bootstrapping again.
    pub fn confirm_bootstrap(&mut self, slot: u64) -> Result<()> {
        ensure!(
            self.clock.is_some_and(|clock| clock.slot == slot) && self.maximum_seen_slot <= slot,
            "Bootstrap requires a coherent snapshot covered by Clock"
        );
        if self.continuous_through != Some(slot) {
            self.continuous_through = Some(slot);
            self.revision = self.revision.wrapping_add(1);
        }
        Ok(())
    }

    pub fn update(&mut self, key: Pubkey, account: CachedAccount) -> Result<bool> {
        let clock = if key == solana_sdk::sysvar::clock::ID {
            ensure!(
                account.owner == solana_sdk::pubkey!("Sysvar1111111111111111111111111111111111111")
                    && account.data.len() >= 40,
                "Invalid Clock account"
            );
            let clock_slot = u64::from_le_bytes(account.data[0..8].try_into()?);
            ensure!(clock_slot == account.slot, "Clock slot does not match stream slot");
            let epoch = u64::from_le_bytes(account.data[16..24].try_into()?);
            let timestamp = u64::try_from(i64::from_le_bytes(account.data[32..40].try_into()?))?;
            Some(ChainClock { slot: account.slot, epoch, unix_timestamp: timestamp })
        } else {
            None
        };
        let slot = account.slot;
        let changed = self.cache.update(key, account)?;
        if changed {
            self.revision = self.revision.wrapping_add(1);
            if self.continuous_through.is_some_and(|watermark| slot < watermark) {
                self.continuous_through = None;
            }
            self.maximum_seen_slot = self.maximum_seen_slot.max(slot);
            if let Some(clock) = clock {
                self.clock = Some(clock);
            }
        }
        Ok(changed)
    }

    #[cfg(feature = "parser-adapter")]
    pub fn update_from_parser_snapshot(
        &mut self,
        event: sol_parser_sdk::accounts::liquidity_snapshot::RawAccountSnapshotEvent,
    ) -> Result<bool> {
        self.update(
            event.account.pubkey,
            CachedAccount {
                owner: event.account.owner,
                data: if event.account.lamports == 0 { Vec::new() } else { event.account.data },
                slot: event.metadata.slot,
                write_version: event.write_version,
            },
        )
    }

    /// Publish only after the caller has verified a complete coherent account batch
    /// and transport continuity. An empty queue alone does not prove completeness.
    /// Clock must cover every observed update.
    /// Repeated Clock/account data cannot renew a frozen snapshot's wall-clock age.
    /// `latest_slot` is a recent trusted stream watermark, not the signal's slot.
    /// After explicit bootstrap confirmation, unchanged accounts remain observed
    /// through Clock's slot. Otherwise individual last-write ages are checked.
    pub fn publish_if_advanced(
        &mut self,
        routing: &PumpSwapRouting,
        latest_slot: u64,
        maximum_slot_age: u64,
    ) -> bool {
        let Some(ChainClock { slot, epoch, unix_timestamp }) = self.clock else { return false };
        if self.maximum_seen_slot > slot
            || self.published_revision == Some(self.revision)
            || latest_slot.checked_sub(slot).is_none_or(|age| age > maximum_slot_age)
        {
            return false;
        }
        let mut snapshot = self.cache.clone();
        if self.continuous_through.is_some() {
            snapshot.observe_through(slot);
            self.continuous_through = Some(slot);
        }
        routing.publish(
            Arc::new(snapshot),
            CacheReadContext { slot: latest_slot, epoch, maximum_slot_age },
            unix_timestamp,
        );
        self.published_revision = Some(self.revision);
        true
    }

    pub fn reset(&mut self, routing: &PumpSwapRouting) {
        *self = Self::default();
        routing.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PumpSwapRoutingConfig;
    fn clock(slot: u64) -> CachedAccount {
        let mut data = vec![0; 40];
        data[0..8].copy_from_slice(&slot.to_le_bytes());
        data[32..40].copy_from_slice(&1_700_000_000i64.to_le_bytes());
        CachedAccount {
            owner: solana_sdk::pubkey!("Sysvar1111111111111111111111111111111111111"),
            data,
            slot,
            write_version: 0,
        }
    }
    #[test]
    fn feed_requires_clock_coverage_and_never_publishes_future_accounts() {
        let routing = PumpSwapRouting::new(PumpSwapRoutingConfig::new(vec![])).unwrap();
        let mut feed = PumpSwapRoutingFeed::default();
        assert!(!feed.publish_if_advanced(&routing, 12, 32));
        feed.update(solana_sdk::sysvar::clock::ID, clock(10)).unwrap();
        assert!(!feed.publish_if_advanced(&routing, 100, 32), "old Clock must not become fresh");
        assert!(!feed.publish_if_advanced(&routing, 9, 32), "future Clock rejected");
        assert!(feed.publish_if_advanced(&routing, 12, 32));
        assert!(!feed.publish_if_advanced(&routing, 12, 32));
        feed.update(
            Pubkey::new_unique(),
            CachedAccount {
                owner: Pubkey::new_unique(),
                data: vec![1],
                slot: 12,
                write_version: 0,
            },
        )
        .unwrap();
        feed.update(solana_sdk::sysvar::clock::ID, clock(11)).unwrap();
        assert!(!feed.publish_if_advanced(&routing, 12, 32));
        feed.update(solana_sdk::sysvar::clock::ID, clock(12)).unwrap();
        assert!(feed.publish_if_advanced(&routing, 12, 32));
        feed.reset(&routing);
        assert!(!feed.publish_if_advanced(&routing, 12, 32));
        let mut invalid = clock(13);
        invalid.slot = 14;
        assert!(feed.update(solana_sdk::sysvar::clock::ID, invalid).is_err());
    }
}
