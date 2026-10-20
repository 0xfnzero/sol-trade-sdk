#![cfg(feature = "parser-adapter")]
use sol_trade_sdk::trading::core::params::PoolTradeHint;

#[test]
fn cross_dex_hints_are_pool_approved_not_platform_filtered() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/cross_dex_stock_routes_20261020.json"))
            .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let rpc = serde_json::from_value(case["rpc"].clone()).unwrap();
        let mut route = sol_parser_sdk::analyze_rpc_transaction_routes(&rpc, &[]).unwrap();
        let conversion = serde_json::from_value(case["conversion_protocol"].clone()).unwrap();
        let leg = route.legs.iter().find(|l| l.protocol == conversion).unwrap().clone();
        assert!(PoolTradeHint::from_transaction_route(&route, &[]).unwrap().is_empty());
        let hints = PoolTradeHint::from_transaction_route(&route, &[leg.pool]).unwrap();
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].pool, leg.pool);
        assert_eq!(Some(hints[0].input_mint), leg.input_mint);
        // Repeated swaps through one pool yield one candidate, not duplicated hops.
        route.legs.push(leg.clone());
        assert_eq!(PoolTradeHint::from_transaction_route(&route, &[leg.pool]).unwrap().len(), 1);
        route.legs.last_mut().unwrap().input_mint = Some(solana_sdk::pubkey::Pubkey::new_unique());
        assert!(PoolTradeHint::from_transaction_route(&route, &[leg.pool]).is_err());
        route.succeeded = false;
        assert!(PoolTradeHint::from_transaction_route(&route, &[leg.pool]).is_err());
    }
}
