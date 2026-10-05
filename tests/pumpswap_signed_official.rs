use serde_json::{json, Value};
use sol_trade_sdk::{
    instruction::utils::pumpswap::PumpSwapFeeBasisPoints, utils::calc::pumpswap::*,
};

#[test]
fn pumpswap_signed_quotes_match_published_official_sdk() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/pumpswap_signed_official_1_19_0.json"))
            .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let number = |name: &str| case[name].as_str().unwrap().parse::<u64>().unwrap();
        let amount = number("amount");
        let base = number("base_reserve");
        let quote = number("quote_reserve");
        let slippage = number("slippage");
        let virtual_reserve = case["virtual"].as_str().unwrap().parse::<i128>().unwrap();
        let fees = PumpSwapFeeBasisPoints::new(20, 5, 30);
        let actual = [
            buy_base_input_internal_with_fees(
                amount,
                slippage,
                base,
                quote,
                virtual_reserve,
                &fees,
            )
            .map(|r| {
                json!({"internalQuoteAmount": r.internal_quote_amount.to_string(),
                    "uiQuote": r.ui_quote.to_string(), "maxQuote": r.max_quote.to_string()})
            }),
            buy_quote_input_internal_with_fees(
                amount,
                slippage,
                base,
                quote,
                virtual_reserve,
                &fees,
            )
            .map(|r| {
                json!({"base": r.base.to_string(),
                    "internalQuoteWithoutFees": r.internal_quote_without_fees.to_string(),
                    "maxQuote": r.max_quote.to_string()})
            }),
            sell_base_input_internal_with_fees(
                amount,
                slippage,
                base,
                quote,
                virtual_reserve,
                &fees,
            )
            .map(|r| {
                json!({"uiQuote": r.ui_quote.to_string(), "minQuote": r.min_quote.to_string(),
                    "internalQuoteAmountOut": r.internal_quote_amount_out.to_string()})
            }),
            sell_quote_input_internal_with_fees(
                amount,
                slippage,
                base,
                quote,
                virtual_reserve,
                &fees,
            )
            .map(|r| {
                json!({"internalRawQuote": r.internal_raw_quote.to_string(),
                    "base": r.base.to_string(), "minQuote": r.min_quote.to_string()})
            }),
        ];
        for (mode, result) in actual.into_iter().enumerate() {
            let expected = &case["results"][mode];
            // SDK 1.19.0 can return negative BN values for quote input too small
            // to cover rounded fees. Such values are not valid on-chain u64 quotes.
            let invalid_u64 = expected.as_object().is_some_and(|fields| {
                fields.values().any(|value| value.as_str().unwrap().parse::<u64>().is_err())
            });
            // Exact-output sell must also fund the protocol/creator fee outflow.
            // The published helper only checks the user output against the vault.
            let unbacked_sell = mode == 3 && !expected.is_null() && !invalid_u64 && {
                let raw = expected["internalRawQuote"].as_str().unwrap().parse::<u64>().unwrap();
                let lp_fee = (raw as u128 * 20).div_ceil(10_000);
                raw as u128 - lp_fee > quote as u128
            };
            if expected.is_null() || invalid_u64 || unbacked_sell {
                assert!(result.is_err(), "official SDK rejected mode {mode}: {case}");
            } else {
                assert_eq!(
                    &result.unwrap_or_else(|e| panic!("mode {mode}: {case}: {e}")),
                    expected,
                    "mode {mode}: {case}"
                );
            }
        }
    }
}
