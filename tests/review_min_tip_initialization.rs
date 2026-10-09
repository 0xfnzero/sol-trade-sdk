//! Public configuration initialization only; HTTP clients are not used to send.
use sol_trade_sdk::swqos::{SwqosConfig, SwqosRegion};
use solana_commitment_config::CommitmentConfig;

#[tokio::test]
async fn decimal_minimum_initialization_preserves_rounding_and_range_boundaries() {
    for (minimum, expected) in
        [(-0.0, 0u64), (0.000_000_000_49, 0), (0.000_000_000_5, 1), (0.000_000_001_49, 1)]
    {
        let config = SwqosConfig::Jito(
            String::new(),
            SwqosRegion::Default,
            Some("http://localhost:1".into()),
            Some(minimum),
        );
        let client = SwqosConfig::get_swqos_client(
            String::new(),
            CommitmentConfig::confirmed(),
            config,
            false,
        )
        .await
        .unwrap();
        assert_eq!(client.configured_min_tip_lamports(), Some(expected));
    }
    let out_of_range = u64::MAX as f64 / 1_000_000_000.0;
    let config = SwqosConfig::Jito(
        String::new(),
        SwqosRegion::Default,
        Some("invalid-url".into()),
        Some(out_of_range),
    );
    let result =
        SwqosConfig::get_swqos_client(String::new(), CommitmentConfig::confirmed(), config, false)
            .await;
    assert!(result.err().unwrap().to_string().contains("min_tip exceeds"));
}
