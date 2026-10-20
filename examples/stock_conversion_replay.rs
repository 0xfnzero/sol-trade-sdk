//! cargo run --features parser-adapter --example stock_conversion_replay
//! Offline identity discovery only. No signing, sending, RPC or live-price claims.
use sol_trade_sdk::{trading::core::params::PoolTradeHint, PumpSwapRoutingConfig};

#[path = "support/replay_input.rs"]
mod replay_input;
fn main() -> anyhow::Result<()> {
    let Some(cases) = replay_input::load(&[include_str!(
        "../tests/fixtures/cross_dex_stock_routes_20261020.json"
    )])?
    else {
        return Ok(());
    };
    let catalog: serde_json::Value =
        serde_json::from_str(include_str!("../src/stock_token_catalog.json"))?;
    let approved = catalog["pools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["pool"].as_str().unwrap().parse())
        .collect::<Result<Vec<_>, _>>()?;
    for mut case in cases {
        let rpc = serde_json::from_value(case["rpc"].take())?;
        let route = sol_parser_sdk::analyze_rpc_transaction_routes(&rpc, &[])?;
        // Approval is pool-specific. Neither Stonk nor Pump imposes a conversion DEX.
        let hints = PoolTradeHint::from_transaction_route(&route, &approved)?;
        let mut config = PumpSwapRoutingConfig::default();
        config.extend_pools(hints.iter().copied())?;
        for hint in hints {
            let leg = route.legs.iter().find(|l| l.pool == hint.pool).unwrap();
            println!(
                "{}",
                serde_json::json!({"signature":route.signature.to_string(),
                "conversion_dex":leg.protocol,"pool":hint.pool.to_string(),
                "input_mint":hint.input_mint.to_string(),"output_mint":hint.output_mint.to_string(),
                "position":leg.position,"identity_only":true})
            );
        }
        // Before routing.prepare(), publish fresh subscribed pool/mint/vault/config/
        // tick/bin state and Clock. Historical amounts never become order amounts.
    }
    Ok(())
}
