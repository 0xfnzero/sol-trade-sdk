use super::{CacheReadContext, CachedAccount, SubscriptionAccountCache};
use base64::{engine::general_purpose::STANDARD, Engine};
use solana_sdk::pubkey::Pubkey;
#[test]
fn actual_bank_hook_mint() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/token_hook_bank_20261008.json"
    ))
    .unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 3);
    for c in cases {
        let owner: Pubkey = c["owner"].as_str().unwrap().parse().unwrap();
        let key = Pubkey::new_unique();
        let mut cache = SubscriptionAccountCache::default();
        cache
            .update(
                key,
                CachedAccount {
                    owner,
                    data: STANDARD.decode(c["data"].as_str().unwrap()).unwrap(),
                    slot: 100,
                    write_version: 0,
                },
            )
            .unwrap();
        let result = cache.mint(
            key,
            Some(owner),
            CacheReadContext { slot: 100, epoch: 0, maximum_slot_age: 0 },
        );
        let rpc_fee = super::super::raydium_cpmm::token_transfer_fee_for_epoch(
            &cache.accounts[&key].data,
            owner,
            0,
        );
        if c["reject_hook"].as_bool().unwrap() {
            assert!(result.unwrap_err().to_string().to_lowercase().contains("hook"));
            assert!(rpc_fee.unwrap_err().to_string().to_lowercase().contains("hook"));
        } else {
            assert_eq!(result.unwrap().1.calculate(10001), 0);
            assert_eq!(rpc_fee.unwrap().calculate(10001), 0);
        }
    }
}
