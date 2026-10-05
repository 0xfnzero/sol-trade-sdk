//! Replay captured account bytes through the same cache used by gRPC updates.
//! Historical fixture bytes came from RPC simulation; these tests perform no RPC.
use anyhow::Result;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::Value;
use sol_trade_sdk::{
    instruction::utils::raydium_cpmm::{accounts, get_creator_fee_share_pda},
    trading::core::params::{CacheReadContext, CachedAccount, SubscriptionAccountCache},
};
use solana_sdk::pubkey::Pubkey;

fn replay(
    fixture: &str,
) -> Result<(SubscriptionAccountCache, Pubkey, Pubkey, CacheReadContext, Value)> {
    let report: Value = serde_json::from_str(fixture)?;
    let r = report["results"][0].clone();
    let pool: Pubkey = r["before_accounts"][0]["pubkey"].as_str().unwrap().parse()?;
    let snapshots = r["before_accounts"].as_array().unwrap();
    let mut cache = SubscriptionAccountCache::default();
    // Use a fixed replay slot: fixture's historical slot is irrelevant to unit tests.
    for a in snapshots.iter().filter(|a| !a.is_null()) {
        cache.update(
            a["pubkey"].as_str().unwrap().parse()?,
            CachedAccount {
                owner: a["owner"].as_str().unwrap().parse()?,
                data: STANDARD.decode(a["data_base64"].as_str().unwrap())?,
                slot: 100,
                write_version: 1,
            },
        )?;
    }
    let raw_pool = &snapshots[0];
    let bytes = STANDARD.decode(raw_pool["data_base64"].as_str().unwrap())?;
    let p = sol_trade_sdk::instruction::utils::raydium_cpmm_types::pool_state_decode(&bytes[8..])
        .unwrap();
    let share = get_creator_fee_share_pda(&p.pool_creator, &p.amm_config);
    // The captured getMultipleAccounts result explicitly proves absence here.
    // A quiet live gRPC stream alone does NOT justify inserting this tombstone.
    if snapshots[2].is_null() {
        cache.update(
            share,
            CachedAccount { owner: Pubkey::default(), data: vec![], slot: 100, write_version: 1 },
        )?;
    }
    Ok((
        cache,
        pool,
        share,
        CacheReadContext { slot: 100, epoch: p.recent_epoch, maximum_slot_age: 10 },
        r,
    ))
}

#[test]
fn cached_preparation_matches_captured_collection_in_both_modes() -> Result<()> {
    for fixture in [
        include_str!("fixtures/cpmm_creator_fee_default_mainnet.json"),
        include_str!("fixtures/cpmm_creator_fee_override_mainnet.json"),
        include_str!("fixtures/cpmm_creator_fee_token2022_mainnet.json"),
    ] {
        let (cache, pool, share, ctx, r) = replay(fixture)?;
        for payer in [None, Some(Pubkey::new_unique())] {
            let prepared = cache.prepare_cpmm_creator_fee_collection(pool, payer, ctx)?;
            assert_eq!(prepared.share_rate, r["share_rate"].as_u64().unwrap());
            assert_eq!(prepared.creator_fee_share, share);
            assert_eq!(prepared.instruction.accounts.len(), if payer.is_some() { 16 } else { 15 });
            assert_eq!(prepared.instruction.accounts.last().unwrap().pubkey, share);
            for (i, payout, protocol) in [
                (0, prepared.creator_payout_token0, prepared.protocol_share_token0),
                (1, prepared.creator_payout_token1, prepared.protocol_share_token1),
            ] {
                let check = &r["checks"][i];
                // Captured creator_before_transfer_fee is gross, before the separate transfer fee.
                assert_eq!(
                    payout.to_string(),
                    check["creator_before_transfer_fee"].as_str().unwrap()
                );
                assert_eq!(protocol.to_string(), check["protocol_share"].as_str().unwrap());
            }
        }
    }
    Ok(())
}

#[test]
fn cache_rejects_stale_future_invalid_and_unknown_share_state() -> Result<()> {
    let (mut cache, pool, share, ctx, _) =
        replay(include_str!("fixtures/cpmm_creator_fee_override_mainnet.json"))?;
    for slot in [99, 111] {
        assert!(cache
            .prepare_cpmm_creator_fee_collection(pool, None, CacheReadContext { slot, ..ctx })
            .is_err());
    }
    // A newer malformed CPMM-owned update is an error, never config fallback.
    cache.update(
        share,
        CachedAccount { owner: accounts::RAYDIUM_CPMM, data: vec![1], slot: 101, write_version: 1 },
    )?;
    assert!(cache
        .prepare_cpmm_creator_fee_collection(pool, None, CacheReadContext { slot: 101, ..ctx })
        .is_err());
    // Closure restores config fallback and still requires the PDA in instruction.
    cache.update(
        share,
        CachedAccount { owner: Pubkey::default(), data: vec![], slot: 102, write_version: 1 },
    )?;
    let p = cache.prepare_cpmm_creator_fee_collection(
        pool,
        None,
        CacheReadContext { slot: 102, ..ctx },
    )?;
    assert_eq!(p.share_rate, 50_000);
    assert_eq!(p.instruction.accounts.last().unwrap().pubkey, share);
    assert!(!cache.update(
        share,
        CachedAccount { owner: accounts::RAYDIUM_CPMM, data: vec![1], slot: 101, write_version: 2 }
    )?);
    assert_eq!(
        cache
            .prepare_cpmm_creator_fee_collection(pool, None, CacheReadContext { slot: 102, ..ctx })?
            .share_rate,
        50_000
    );
    Ok(())
}

#[test]
fn missing_stream_update_is_not_proof_of_pda_absence() -> Result<()> {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/cpmm_creator_fee_default_mainnet.json"))?;
    let rows = report["results"][0]["before_accounts"].as_array().unwrap();
    let mut cache = SubscriptionAccountCache::default();
    for a in &rows[..2] {
        cache.update(
            a["pubkey"].as_str().unwrap().parse()?,
            CachedAccount {
                owner: a["owner"].as_str().unwrap().parse()?,
                data: STANDARD.decode(a["data_base64"].as_str().unwrap())?,
                slot: 100,
                write_version: 1,
            },
        )?;
    }
    let error = cache
        .prepare_cpmm_creator_fee_collection(
            rows[0]["pubkey"].as_str().unwrap().parse()?,
            None,
            CacheReadContext { slot: 100, epoch: 0, maximum_slot_age: 10 },
        )
        .unwrap_err();
    assert!(error.to_string().contains("Unknown cached CreatorFeeShare PDA"), "{error}");
    Ok(())
}

#[cfg(feature = "parser-adapter")]
#[test]
fn grpc_snapshot_adapter_applies_rate_updates_and_closures_in_version_order() -> Result<()> {
    use sol_parser_sdk::{
        accounts::{liquidity_snapshot::RawAccountSnapshotEvent, AccountData},
        core::events::EventMetadata,
    };
    let (mut cache, pool, share, ctx, r) =
        replay(include_str!("fixtures/cpmm_creator_fee_override_mainnet.json"))?;
    let raw = &r["before_accounts"][2];
    let mut data = STANDARD.decode(raw["data_base64"].as_str().unwrap())?;
    data[73..81].copy_from_slice(&1_000_000u64.to_le_bytes());
    let mut event = RawAccountSnapshotEvent {
        metadata: EventMetadata { slot: 101, ..Default::default() },
        account: AccountData {
            pubkey: share,
            owner: accounts::RAYDIUM_CPMM,
            data,
            lamports: 1,
            executable: false,
            rent_epoch: 0,
        },
        write_version: 2,
        is_startup: false,
    };
    assert!(cache.update_from_parser_snapshot(&event)?);
    let full = cache.prepare_cpmm_creator_fee_collection(
        pool,
        None,
        CacheReadContext { slot: 101, ..ctx },
    )?;
    assert_eq!(full.share_rate, 1_000_000);
    assert_eq!((full.creator_payout_token0, full.creator_payout_token1), (0, 0));
    assert!(!cache.update_from_parser_snapshot(&event)?);
    event.write_version = 3;
    event.account.lamports = 0; // retained bytes must not keep the old custom rate alive.
    assert!(cache.update_from_parser_snapshot(&event)?);
    assert_eq!(
        cache
            .prepare_cpmm_creator_fee_collection(pool, None, CacheReadContext { slot: 101, ..ctx })?
            .share_rate,
        50_000
    );
    event.write_version = 2;
    event.account.lamports = 1;
    assert!(!cache.update_from_parser_snapshot(&event)?);
    assert_eq!(
        cache
            .prepare_cpmm_creator_fee_collection(pool, None, CacheReadContext { slot: 101, ..ctx })?
            .share_rate,
        50_000
    );
    Ok(())
}

/// These JSON lines were received from Yellowstone on October 5, 2026, without RPC.
/// This partial capture contains config updates, not a complete pool snapshot.
#[cfg(feature = "parser-adapter")]
#[test]
fn captured_grpc_config_updates_decode_and_do_not_fake_collection_readiness() -> Result<()> {
    use sol_parser_sdk::{
        accounts::{liquidity_snapshot::RawAccountSnapshotEvent, parse_account_unified},
        DexEvent,
    };
    use sol_trade_sdk::instruction::utils::raydium_cpmm_types::amm_config_decode;
    let mut cache = SubscriptionAccountCache::default();
    let mut slot = 0;
    let mut count = 0;
    for line in include_str!("fixtures/cpmm_creator_fee_grpc_config_mainnet.jsonl").lines() {
        let e: RawAccountSnapshotEvent = serde_json::from_str(line)?;
        assert_eq!(e.account.data.len(), 236);
        let trade = amm_config_decode(&e.account.data[8..]).unwrap();
        let DexEvent::RaydiumCpmmAmmConfigAccount(parsed) =
            parse_account_unified(&e.account, e.metadata.clone(), None).unwrap()
        else {
            panic!("config")
        };
        assert_eq!(serde_json::to_value(trade)?, serde_json::to_value(parsed.amm_config)?);
        slot = slot.max(e.metadata.slot);
        cache.update_from_parser_snapshot(&e)?;
        count += 1;
    }
    assert!(count > 0);
    let pool: Pubkey = "EjqGHevzN2pgaTf5CZnbMBuiYdPpWYx4pmeY7abwqyrM".parse()?;
    let error = cache
        .prepare_cpmm_creator_fee_collection(
            pool,
            None,
            CacheReadContext { slot, epoch: 0, maximum_slot_age: 256 },
        )
        .unwrap_err();
    assert!(error.to_string().contains("Missing cached account"), "{error}");
    Ok(())
}

#[test]
fn cached_collection_rejects_cleared_fees_before_building_instruction() -> Result<()> {
    let (mut cache, pool, _, ctx, r) =
        replay(include_str!("fixtures/cpmm_creator_fee_default_mainnet.json"))?;
    let raw = &r["before_accounts"][0];
    let mut data = STANDARD.decode(raw["data_base64"].as_str().unwrap())?;
    // Account offset 397: creator_fees_token0/1 follow creator_fee_on,
    // enable_creator_fee and six reserved bytes in the unchanged PoolState layout.
    let before =
        sol_trade_sdk::instruction::utils::raydium_cpmm_types::pool_state_decode(&data[8..])
            .unwrap();
    assert_eq!(u64::from_le_bytes(data[397..405].try_into().unwrap()), before.creator_fees_token0);
    assert_eq!(u64::from_le_bytes(data[405..413].try_into().unwrap()), before.creator_fees_token1);
    data[397..413].fill(0);
    cache.update(
        pool,
        CachedAccount { owner: accounts::RAYDIUM_CPMM, data, slot: 101, write_version: 1 },
    )?;
    let error = cache
        .prepare_cpmm_creator_fee_collection(pool, None, CacheReadContext { slot: 101, ..ctx })
        .unwrap_err();
    assert!(error.to_string().contains("No accrued CPMM creator fees"), "{error}");
    Ok(())
}

#[cfg(feature = "parser-adapter")]
#[test]
fn parser_account_and_raw_snapshot_adapters_agree_on_closed_share_accounts() -> Result<()> {
    use sol_parser_sdk::accounts::AccountData;
    let (mut cache, pool, share, ctx, r) =
        replay(include_str!("fixtures/cpmm_creator_fee_override_mainnet.json"))?;
    let raw = &r["before_accounts"][2];
    let account = AccountData {
        pubkey: share,
        owner: accounts::RAYDIUM_CPMM,
        data: STANDARD.decode(raw["data_base64"].as_str().unwrap())?,
        lamports: 0,
        executable: false,
        rent_epoch: 0,
    };
    // Retained bytes still encode a zero custom rate; a zero-lamport closure must
    // discard them in both public ingestion APIs and restore config fallback.
    assert!(cache.update_from_parser_account(&account, 101, 2)?);
    assert_eq!(
        cache
            .prepare_cpmm_creator_fee_collection(pool, None, CacheReadContext { slot: 101, ..ctx })?
            .share_rate,
        50_000
    );
    let mut delayed = account.clone();
    delayed.lamports = 1;
    assert!(!cache.update_from_parser_account(&delayed, 100, 3)?);
    assert_eq!(
        cache
            .prepare_cpmm_creator_fee_collection(pool, None, CacheReadContext { slot: 101, ..ctx })?
            .share_rate,
        50_000
    );
    Ok(())
}

#[test]
fn prepared_collection_versions_reject_updated_state_even_with_same_estimate() -> Result<()> {
    let (mut cache, pool, share, ctx, r) =
        replay(include_str!("fixtures/cpmm_creator_fee_default_mainnet.json"))?;
    let prepared = cache.prepare_cpmm_creator_fee_collection(pool, None, ctx)?;
    assert_eq!(prepared.snapshot_slot, 100);
    assert_eq!(prepared.account_versions[0], (pool, 100, 1));
    assert_eq!(prepared.account_versions[2], (share, 100, 1));
    cache.validate_cpmm_creator_fee_collection(
        &prepared,
        None,
        CacheReadContext { slot: 101, ..ctx },
    )?;
    let raw = &r["before_accounts"][1];
    // Config bytes can be unchanged while its observed version advances. A staged
    // preparation must still be refreshed rather than silently reusing metadata.
    cache.update(
        prepared.amm_config,
        CachedAccount {
            owner: accounts::RAYDIUM_CPMM,
            data: STANDARD.decode(raw["data_base64"].as_str().unwrap())?,
            slot: 101,
            write_version: 2,
        },
    )?;
    let error = cache
        .validate_cpmm_creator_fee_collection(
            &prepared,
            None,
            CacheReadContext { slot: 101, ..ctx },
        )
        .unwrap_err();
    assert!(error.to_string().contains("preparation changed"), "{error}");
    let refreshed = cache.prepare_cpmm_creator_fee_collection(
        pool,
        None,
        CacheReadContext { slot: 101, ..ctx },
    )?;
    assert_eq!(refreshed.creator_payout_token0, prepared.creator_payout_token0);
    assert_eq!(refreshed.account_versions[1], (prepared.amm_config, 101, 2));
    cache.validate_cpmm_creator_fee_collection(
        &refreshed,
        None,
        CacheReadContext { slot: 101, ..ctx },
    )?;
    Ok(())
}

#[test]
fn staged_collection_validation_rejects_stale_context_wrong_payer_and_modified_amounts(
) -> Result<()> {
    let (cache, pool, _, ctx, _) =
        replay(include_str!("fixtures/cpmm_creator_fee_default_mainnet.json"))?;
    let payer = Pubkey::new_unique();
    let prepared = cache.prepare_cpmm_creator_fee_collection(pool, Some(payer), ctx)?;
    cache.validate_cpmm_creator_fee_collection(&prepared, Some(payer), ctx)?;
    for invalid_slot in [99, 111] {
        assert!(cache
            .validate_cpmm_creator_fee_collection(
                &prepared,
                Some(payer),
                CacheReadContext { slot: invalid_slot, ..ctx }
            )
            .is_err());
    }
    assert!(cache.validate_cpmm_creator_fee_collection(&prepared, None, ctx).is_err());
    assert!(cache
        .validate_cpmm_creator_fee_collection(&prepared, Some(Pubkey::new_unique()), ctx)
        .is_err());
    let mut impossible_snapshot = prepared.clone();
    impossible_snapshot.snapshot_slot = 99;
    assert!(cache
        .validate_cpmm_creator_fee_collection(&impossible_snapshot, Some(payer), ctx)
        .is_err());
    let mut changed = prepared;
    changed.creator_payout_token0 += 1;
    assert!(cache.validate_cpmm_creator_fee_collection(&changed, Some(payer), ctx).is_err());
    Ok(())
}

#[cfg(feature = "parser-adapter")]
#[test]
fn owned_snapshot_matches_borrowed_adapter_and_rejects_conflicting_replays() -> Result<()> {
    use sol_parser_sdk::{
        accounts::{liquidity_snapshot::RawAccountSnapshotEvent, AccountData},
        core::events::EventMetadata,
    };
    let key = Pubkey::new_unique();
    let owner = Pubkey::new_unique();
    let event = RawAccountSnapshotEvent {
        metadata: EventMetadata { slot: 100, ..Default::default() },
        account: AccountData {
            pubkey: key,
            owner,
            data: vec![1; 236],
            executable: false,
            lamports: 1,
            rent_epoch: 0,
        },
        write_version: 1,
        is_startup: false,
    };
    let mut borrowed = SubscriptionAccountCache::default();
    let mut owned = SubscriptionAccountCache::default();
    assert!(borrowed.update_from_parser_snapshot(&event)?);
    assert!(owned.update_from_owned_parser_snapshot(event.clone())?);
    assert!(!borrowed.update_from_parser_snapshot(&event)?);
    assert!(!owned.update_from_owned_parser_snapshot(event.clone())?);
    let mut conflicting = event.clone();
    conflicting.account.data[0] = 2;
    assert!(borrowed.update_from_parser_snapshot(&conflicting).is_err());
    assert!(owned.update_from_owned_parser_snapshot(conflicting).is_err());
    let mut closed = event;
    closed.account.lamports = 0;
    closed.write_version = 2;
    assert!(borrowed.update_from_parser_snapshot(&closed)?);
    assert!(owned.update_from_owned_parser_snapshot(closed)?);
    // Both normalize residual closed-account bytes to the same empty observation.
    let empty = CachedAccount { owner, data: vec![], slot: 100, write_version: 2 };
    assert!(!borrowed.update(key, empty.clone())?);
    assert!(!owned.update(key, empty)?);
    Ok(())
}

/// Local CPU measurement only; no network, RPC, transaction submission or assertion
/// of end-to-end latency. Run with --release --ignored --nocapture.
#[test]
#[ignore = "explicit local latency measurement"]
fn cached_collection_latency() -> Result<()> {
    use std::{hint::black_box, time::Instant};
    let (cache, pool, _, ctx, _) =
        replay(include_str!("fixtures/cpmm_creator_fee_override_mainnet.json"))?;
    for payer in [None, Some(Pubkey::new_unique())] {
        for _ in 0..1000 {
            black_box(cache.prepare_cpmm_creator_fee_collection(pool, payer, ctx)?);
        }
        let prepared = cache.prepare_cpmm_creator_fee_collection(pool, payer, ctx)?;
        for validation in [false, true] {
            let mut samples = Vec::with_capacity(10000);
            for _ in 0..10000 {
                let start = Instant::now();
                if validation {
                    cache.validate_cpmm_creator_fee_collection(black_box(&prepared), payer, ctx)?;
                } else {
                    black_box(cache.prepare_cpmm_creator_fee_collection(
                        black_box(pool),
                        payer,
                        ctx,
                    )?);
                }
                samples.push(start.elapsed().as_nanos());
            }
            samples.sort_unstable();
            println!(
                "permissionless={} validation={} samples={} p50_ns={} p99_ns={} rpc_calls=0",
                payer.is_some(),
                validation,
                samples.len(),
                samples[5000],
                samples[9900]
            );
        }
    }
    Ok(())
}

#[cfg(feature = "parser-adapter")]
#[test]
fn borrowed_ingestion_preserves_stale_and_conflict_semantics_before_copying() -> Result<()> {
    use sol_parser_sdk::accounts::AccountData;
    let mut cache = SubscriptionAccountCache::default();
    let mut account = AccountData {
        pubkey: Pubkey::new_unique(),
        owner: accounts::RAYDIUM_CPMM,
        data: vec![1; 637],
        lamports: 1,
        rent_epoch: 0,
        executable: false,
    };
    assert!(cache.update_from_parser_account(&account, 100, 5)?);
    assert!(!cache.update_from_parser_account(&account, 100, 5)?);
    account.owner = Pubkey::default();
    assert!(cache.update_from_parser_account(&account, 100, 5).is_err());
    // Lower slot wins over a higher write version; stale bytes/owner are ignored.
    assert!(!cache.update_from_parser_account(&account, 99, u64::MAX)?);
    account.owner = accounts::RAYDIUM_CPMM;
    account.data[0] = 2;
    assert!(cache.update_from_parser_account(&account, 100, 5).is_err());
    account.lamports = 0;
    assert!(cache.update_from_parser_account(&account, 101, 1)?);
    account.data.fill(3);
    assert!(!cache.update_from_parser_account(&account, 101, 1)?);
    // A higher slot is a newer observation even when write_version restarts.
    account.lamports = 1;
    assert!(cache.update_from_parser_account(&account, 102, 0)?);
    Ok(())
}

#[cfg(feature = "parser-adapter")]
#[test]
#[ignore = "explicit local replay ingestion benchmark"]
fn borrowed_replay_latency() -> Result<()> {
    use sol_parser_sdk::accounts::AccountData;
    use std::{hint::black_box, time::Instant};
    let account = AccountData {
        pubkey: Pubkey::new_unique(),
        owner: accounts::RAYDIUM_CPMM,
        data: vec![1; 637],
        lamports: 1,
        rent_epoch: 0,
        executable: false,
    };
    let mut cache = SubscriptionAccountCache::default();
    cache.update_from_parser_account(&account, 100, 5)?;
    for stale in [false, true] {
        for clone_first in [true, false] {
            let slot = if stale { 99 } else { 100 };
            let mut samples = Vec::with_capacity(1000);
            for _ in 0..1000 {
                let start = Instant::now();
                for _ in 0..100 {
                    let updated = if clone_first {
                        cache.update(
                            account.pubkey,
                            black_box(CachedAccount {
                                owner: account.owner,
                                data: account.data.clone(),
                                slot,
                                write_version: 5,
                            }),
                        )?
                    } else {
                        cache.update_from_parser_account(black_box(&account), slot, 5)?
                    };
                    assert!(!updated);
                }
                samples.push(start.elapsed().as_nanos());
            }
            samples.sort_unstable();
            println!("stale={stale} clone_first={clone_first} bytes=637 samples=1000 batch_size=100 p50_ns={} p99_ns={} rpc_calls=0", samples[500] as f64 / 100.0, samples[990] as f64 / 100.0);
        }
    }
    Ok(())
}

#[test]
#[ignore = "explicit local invalidated preparation benchmark"]
fn invalidated_collection_latency() -> Result<()> {
    use std::{hint::black_box, time::Instant};
    let (mut cache, pool, _, ctx, row) =
        replay(include_str!("fixtures/cpmm_creator_fee_override_mainnet.json"))?;
    let prepared = cache.prepare_cpmm_creator_fee_collection(pool, None, ctx)?;
    cache.update(
        prepared.amm_config,
        CachedAccount {
            owner: accounts::RAYDIUM_CPMM,
            data: STANDARD.decode(row["before_accounts"][1]["data_base64"].as_str().unwrap())?,
            slot: ctx.slot,
            write_version: 2,
        },
    )?;
    for reconstruct_first in [true, false] {
        let mut samples = Vec::with_capacity(1000);
        for _ in 0..1000 {
            let start = Instant::now();
            for _ in 0..100 {
                let rejected = if reconstruct_first {
                    let current =
                        black_box(cache.prepare_cpmm_creator_fee_collection(pool, None, ctx)?);
                    current != *black_box(&prepared)
                } else {
                    black_box(cache.validate_cpmm_creator_fee_collection(
                        black_box(&prepared),
                        None,
                        ctx,
                    ))
                    .is_err()
                };
                assert!(rejected);
            }
            samples.push(start.elapsed().as_nanos());
        }
        samples.sort_unstable();
        println!("invalidated=true reconstruct_first={reconstruct_first} samples=1000 batch_size=100 p50_ns={} p99_ns={} rpc_calls=0", samples[500] as f64 / 100.0, samples[990] as f64 / 100.0);
    }
    Ok(())
}

#[test]
fn cached_collection_checks_protocol_booking_overflow_for_both_tokens() -> Result<()> {
    use sol_trade_sdk::instruction::utils::raydium_cpmm::split_creator_fee;
    let (cache, pool, _, ctx, report) =
        replay(include_str!("fixtures/cpmm_creator_fee_default_mainnet.json"))?;
    let rate = cache.prepare_cpmm_creator_fee_collection(pool, None, ctx)?.share_rate;
    let protocol_share = split_creator_fee(1000, rate)?.1;
    assert!(protocol_share > 0);
    let raw = &report["before_accounts"][0];
    let original = STANDARD.decode(raw["data_base64"].as_str().unwrap())?;
    for offset in [341, 349] {
        for overflow in [false, true] {
            let mut trial = cache.clone();
            let mut bytes = original.clone();
            bytes[397..405].copy_from_slice(&1000u64.to_le_bytes());
            bytes[405..413].copy_from_slice(&1000u64.to_le_bytes());
            let counter = u64::MAX - protocol_share + u64::from(overflow);
            bytes[offset..offset + 8].copy_from_slice(&counter.to_le_bytes());
            trial.update(
                pool,
                CachedAccount {
                    owner: accounts::RAYDIUM_CPMM,
                    data: bytes,
                    slot: 101,
                    write_version: 1,
                },
            )?;
            for payer in [None, Some(Pubkey::new_unique())] {
                let result = trial.prepare_cpmm_creator_fee_collection(
                    pool,
                    payer,
                    CacheReadContext { slot: 101, ..ctx },
                );
                if overflow {
                    assert!(result.unwrap_err().to_string().contains("protocol fee overflow"));
                } else {
                    let prepared = result?;
                    assert_eq!(prepared.protocol_share_token0, protocol_share);
                    assert_eq!(prepared.protocol_share_token1, protocol_share);
                }
            }
        }
    }
    // An explicit zero override adds nothing, even to an already-maximal counter.
    let (mut zero_cache, zero_pool, _, zero_ctx, zero_report) =
        replay(include_str!("fixtures/cpmm_creator_fee_override_mainnet.json"))?;
    assert_eq!(
        zero_cache.prepare_cpmm_creator_fee_collection(zero_pool, None, zero_ctx)?.share_rate,
        0
    );
    let mut bytes =
        STANDARD.decode(zero_report["before_accounts"][0]["data_base64"].as_str().unwrap())?;
    bytes[341..349].copy_from_slice(&u64::MAX.to_le_bytes());
    bytes[349..357].copy_from_slice(&u64::MAX.to_le_bytes());
    zero_cache.update(
        zero_pool,
        CachedAccount { owner: accounts::RAYDIUM_CPMM, data: bytes, slot: 101, write_version: 1 },
    )?;
    let prepared = zero_cache.prepare_cpmm_creator_fee_collection(
        zero_pool,
        None,
        CacheReadContext { slot: 101, ..zero_ctx },
    )?;
    assert_eq!((prepared.protocol_share_token0, prepared.protocol_share_token1), (0, 0));
    Ok(())
}

#[test]
fn cached_collection_rejects_unsupported_token_programs_and_invalid_mint_pairs() -> Result<()> {
    let (cache, pool, _, ctx, report) =
        replay(include_str!("fixtures/cpmm_creator_fee_default_mainnet.json"))?;
    let original =
        STANDARD.decode(report["before_accounts"][0]["data_base64"].as_str().unwrap())?;
    for case in 0..5 {
        let mut trial = cache.clone();
        let mut bytes = original.clone();
        let expected = if case < 2 {
            let offset = if case == 0 { 232 } else { 264 };
            bytes[offset..offset + 32].copy_from_slice(Pubkey::new_unique().as_ref());
            "Unsupported CPMM token program"
        } else {
            if case == 2 {
                let mint = bytes[168..200].to_vec();
                bytes[200..232].copy_from_slice(&mint);
            } else {
                let offset = if case == 3 { 168 } else { 200 };
                bytes[offset..offset + 32].fill(0);
            }
            "Invalid CPMM mint pair"
        };
        trial.update(
            pool,
            CachedAccount {
                owner: accounts::RAYDIUM_CPMM,
                data: bytes,
                slot: 101,
                write_version: 1,
            },
        )?;
        for payer in [None, Some(Pubkey::new_unique())] {
            let error = trial
                .prepare_cpmm_creator_fee_collection(
                    pool,
                    payer,
                    CacheReadContext { slot: 101, ..ctx },
                )
                .unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
        }
    }
    Ok(())
}
