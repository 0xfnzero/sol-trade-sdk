use base64::{engine::general_purpose::STANDARD, Engine};
use sol_trade_sdk::trading::core::params::{CachedAccount, PoolTradeHint};
use sol_trade_sdk::{PumpSwapRouting, PumpSwapRoutingConfig, PumpSwapRoutingFeed};

#[test]
fn continuous_subscription_keeps_unchanged_accounts_and_publishes_same_slot_writes(
) -> Result<(), Box<dyn std::error::Error>> {
    let f: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/pumpswap_ttwo_route_accounts.json"))?;
    let slot = f["slot"].as_u64().unwrap();
    let epoch = f["epoch"].as_u64().unwrap();
    let time = f["unix_timestamp"].as_u64().unwrap();
    let mut feed = PumpSwapRoutingFeed::default();
    for a in f["accounts"].as_array().unwrap() {
        feed.update(
            a["address"].as_str().unwrap().parse()?,
            CachedAccount {
                owner: a["owner"].as_str().unwrap().parse()?,
                data: STANDARD.decode(a["data_base64"].as_str().unwrap())?,
                slot: a["slot"].as_u64().unwrap(),
                write_version: 0,
            },
        )?;
    }
    let s = &f["route_search"];
    let hints = s["conversion_pools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| PoolTradeHint {
            pool: p["pool"].as_str().unwrap().parse().unwrap(),
            input_mint: p["input_mint"].as_str().unwrap().parse().unwrap(),
            output_mint: p["output_mint"].as_str().unwrap().parse().unwrap(),
        })
        .collect();
    let routing = PumpSwapRouting::new(PumpSwapRoutingConfig::new(hints))?;
    let clock_key = "SysvarC1ock11111111111111111111111111111111".parse()?;
    let clock = |slot: u64| {
        let mut data = vec![0; 40];
        data[..8].copy_from_slice(&slot.to_le_bytes());
        data[16..24].copy_from_slice(&epoch.to_le_bytes());
        data[32..40].copy_from_slice(&time.to_le_bytes());
        CachedAccount {
            owner: "Sysvar1111111111111111111111111111111111111".parse().unwrap(),
            data,
            slot,
            write_version: 1,
        }
    };
    feed.update(clock_key, clock(slot))?;
    feed.confirm_bootstrap(slot)?;
    assert!(feed.publish_if_advanced(&routing, slot, 32));
    let prepare = || {
        routing.prepare(
            s["pool"].as_str().unwrap().parse().unwrap(),
            s["token_mint"].as_str().unwrap().parse().unwrap(),
            s["asset_mint"].as_str().unwrap().parse().unwrap(),
            1_000_000,
            100,
            true,
        )
    };
    assert!(prepare().is_ok(), "fixture must initially quote");
    let pool_row =
        f["accounts"].as_array().unwrap().iter().find(|a| a["address"] == s["pool"]).unwrap();
    let updated = CachedAccount {
        owner: pool_row["owner"].as_str().unwrap().parse()?,
        data: STANDARD.decode(pool_row["data_base64"].as_str().unwrap())?,
        slot,
        write_version: 2,
    };
    assert!(feed.update(s["pool"].as_str().unwrap().parse()?, updated)?);
    assert!(feed.publish_if_advanced(&routing, slot, 32));
    assert!(!feed.publish_if_advanced(&routing, slot, 32));
    // A complete incremental stream has no writes to these unchanged accounts.
    feed.update(clock_key, clock(slot + 33))?;
    assert!(feed.publish_if_advanced(&routing, slot + 33, 32));
    assert!(prepare().is_ok(), "unchanged accounts must remain usable after 33 slots");
    assert!(!feed.update(clock_key, clock(slot + 33))?);
    assert!(!feed.publish_if_advanced(&routing, slot + 33, 32));
    assert!(!feed.publish_if_advanced(&routing, slot + 100, 32));
    feed.reset(&routing);
    assert!(prepare().is_err());
    feed.update(clock_key, clock(slot + 34))?;
    assert!(feed.publish_if_advanced(&routing, slot + 34, 32));
    assert!(prepare().is_err(), "a new Clock must not restore missing bootstrap accounts");
    Ok(())
}
