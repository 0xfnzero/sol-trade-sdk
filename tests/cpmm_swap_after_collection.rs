#[path = "../examples/support/cpmm_swap_collection_simulation.rs"]
mod simulation;
#[tokio::test]
#[ignore = "requires mainnet RPC and creator token1 balance to virtually fund a trader"]
async fn mainnet_swap_is_unchanged_after_creator_fee_collection() {
    let report = simulation::run_from_env().await.expect("mainnet collection/swap comparison");
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    assert_eq!(report["comparisons"].as_array().unwrap().len(), 4);
}

#[test]
fn captured_mainnet_swaps_quotes_logs_and_fee_counters_match() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/cpmm_swap_after_collection_mainnet.json"))
            .unwrap();
    simulation::verify_saved_report(&report).expect("raw mainnet swap fixture validation");
}
