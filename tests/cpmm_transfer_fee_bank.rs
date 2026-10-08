//! Audited deployed CPMM local-bank snapshots with nonzero Token-2022 fees.
use anyhow::Result;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::Value;
use sol_trade_sdk::trading::core::params::{CacheReadContext,CachedAccount,CachedQuoteRequest,PoolTradeHint,SubscriptionAccountCache};
#[test]
fn nonzero_transfer_fee_quotes_match_three_native_sdks_and_executed_bank() -> Result<()> {
 let fixture:Value=serde_json::from_str(include_str!("fixtures/cpmm_transfer_fee_bank_20261008.json"))?;
 for case in fixture["cases"].as_array().unwrap(){
  let s=&case["snapshot"];let v=&case["validation"];let mut cache=SubscriptionAccountCache::default();
  for a in s["accounts"].as_array().unwrap(){cache.update(a["pubkey"].as_str().unwrap().parse()?,CachedAccount{owner:a["owner"].as_str().unwrap().parse()?,data:STANDARD.decode(a["data"].as_str().unwrap())?,slot:a["slot"].as_str().unwrap().parse()?,write_version:0})?;}
  let leg=&s["legs"][0];let hint=PoolTradeHint{pool:leg["pool"].as_str().unwrap().parse()?,input_mint:leg["input_mint"].as_str().unwrap().parse()?,output_mint:leg["output_mint"].as_str().unwrap().parse()?};
  let ctx=CacheReadContext{slot:s["read_slot"].as_str().unwrap().parse()?,epoch:s["epoch"].as_str().unwrap().parse()?,maximum_slot_age:0};
  let quote=cache.quote_exact_in(hint,CachedQuoteRequest{amount_in:s["amount"].as_str().unwrap().parse()?,slippage_basis_points:1000,unix_timestamp:s["unix_timestamp"].as_str().unwrap().parse()?,maximum_arrays:8},ctx)?;
  assert_eq!(quote.estimated_net_amount_out,v["net_credit"].as_u64().unwrap(),"{}",v["name"]);
  assert_eq!(quote.minimum_net_amount_out,v["minimum_net_out"].as_u64().unwrap(),"{}",v["name"]);
 }
 Ok(())
}
