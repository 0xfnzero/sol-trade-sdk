//! Independent SPL oracle for net-output fee recovery, including unreachable u64 targets.
use sol_trade_sdk::trading::core::params::TokenTransferFee;
use spl_token_2022_interface::extension::transfer_fee::TransferFee;

#[test]
fn inverse_fee_boundaries_match_official_spl_and_u64_reachability() {
    let mut samples = Vec::new();
    for bps in [0u16, 1, 300, 9_999, 10_000] {
        for maximum in [0u64, 1, 100, u64::MAX] {
            let official = TransferFee { epoch: 0.into(), maximum_fee: maximum.into(),
                transfer_fee_basis_points: bps.into() };
            let sdk = TokenTransferFee { basis_points: bps, maximum_fee: maximum };
            for amount in [0u64, 1, 99, 100, 101, u64::MAX - 100, u64::MAX] {
                let fee = official.calculate_fee(amount).unwrap();
                assert_eq!(sdk.calculate(amount), fee);
                let inverse = sdk.calculate_inverse(amount);
                let reachable_inverse = amount.checked_add(inverse).map(|_| inverse);
                assert_eq!(reachable_inverse, official.calculate_inverse_fee(amount),
                    "bps={bps} cap={maximum} net={amount}");
                if let Some(gross) = amount.checked_add(inverse) {
                    assert_eq!(gross - official.calculate_fee(gross).unwrap(), amount);
                }
                samples.push(serde_json::json!({"basisPoints":bps,"maximumFee":maximum.to_string(),
                    "amount":amount.to_string(),"fee":fee.to_string(),
                    "inverseFee":official.calculate_inverse_fee(amount).map(|v|v.to_string())}));
            }
        }
    }
    if let Ok(path) = std::env::var("REVIEW_INVERSE_FEE_ORACLE_OUTPUT") {
        let artifact = serde_json::json!({"oracle":"spl_token_2022_interface::extension::transfer_fee::TransferFee",
            "scope":"official forward/inverse arithmetic; null inverse denotes unreachable u64 gross, not zero fee",
            "samples":samples});
        std::fs::write(path, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();
    }
}
