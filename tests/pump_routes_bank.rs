use sol_trade_sdk::instruction::{pump_compact_accounts::{derive_pump_multi_hop_accounts,PumpMultiHop,PumpMultiHopVenue},pump_upgrade::build_pump_upgrade_instruction};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
#[test]
fn native_two_hop_wires_match_deployed_bank() {
 let f:serde_json::Value=serde_json::from_str(include_str!("fixtures/pump_routes_bank_20261009.json")).unwrap();
 for c in f["cases"].as_array().unwrap(){
  let pk=|v:&serde_json::Value|Pubkey::from_str(v.as_str().unwrap()).unwrap();
  let hops:Vec<_>=c["hops"].as_array().unwrap().iter().map(|h|PumpMultiHop{venue:if h["venue"]=="curve"{PumpMultiHopVenue::Curve}else{PumpMultiHopVenue::Pool},base_mint:pk(&h["base_mint"]),quote_mint:pk(&h["quote_mint"]),address:pk(&h["address"]),base_vault:pk(&h["base_vault"]),quote_vault:pk(&h["quote_vault"]),base_token_program:pk(&h["base_token_program"]),quote_token_program:pk(&h["quote_token_program"]),creator:pk(&h["creator"]),index:0,mayhem:false,cashback:false,complete:false}).collect();
  let(fixed,remaining)=derive_pump_multi_hop_accounts(pk(&c["user"]),pk(&c["input_mint"]),pk(&c["output_mint"]),pk(&c["buyback_recipient"]),&hops,false).unwrap();
  let args:Vec<u64>=c["args"].as_array().unwrap().iter().map(|v|v.as_str().unwrap().parse().unwrap()).collect();let ix=build_pump_upgrade_instruction("pump_amm_multi_hop_swap",&fixed,&args,None,&remaining).unwrap();
  assert_eq!(ix.program_id.to_string(),c["program"].as_str().unwrap());let data:String=ix.data.iter().map(|b|format!("{b:02x}")).collect();assert_eq!(data,c["data"].as_str().unwrap());let keys:Vec<_>=ix.accounts.iter().map(|a|a.pubkey.to_string()).collect();let want:Vec<_>=c["accounts"].as_array().unwrap().iter().map(|v|v.as_str().unwrap().to_owned()).collect();assert_eq!(keys,want,"{}",c["name"]);
 }
}
