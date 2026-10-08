use base64::{engine::general_purpose::STANDARD, Engine};
use sol_trade_sdk::instruction::token2022_hook::{
    resolve_hook_accounts, resolve_hook_accounts_with_context,
};
use solana_sdk::pubkey::Pubkey;
use std::{collections::HashMap, str::FromStr};
#[test]
fn executed_dlmm_hook_context() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/dlmm_hook_context_20261009.json")).unwrap();
    for c in fixture["cases"].as_array().unwrap() {
        let pk = |k: &str| Pubkey::from_str(c[k].as_str().unwrap()).unwrap();
        let dec = |k: &str| STANDARD.decode(c[k].as_str().unwrap()).unwrap();
        let hook = pk("hook");
        let mint = pk("mint");
        let owner = pk("mint_owner");
        let meta = pk("meta");
        let metaowner = pk("meta_owner");
        let mintdata = dec("mint_data");
        let metadata = dec("meta_data");
        let data = dec("execute_data");
        let keys: Vec<_> = c["execute_accounts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| Pubkey::from_str(k.as_str().unwrap()).unwrap())
            .collect();
        let accounts: HashMap<_, _> = c["account_data"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| {
                (Pubkey::from_str(k).unwrap(), STANDARD.decode(v.as_str().unwrap()).unwrap())
            })
            .collect();
        let invoke = |m: &[u8], d: &[u8], a: &HashMap<Pubkey, Vec<u8>>| {
            resolve_hook_accounts_with_context(
                &hook, &mint, &owner, &mintdata, &meta, &metaowner, m, &keys, d, a,
            )
        };
        let got = invoke(&metadata, &data, &accounts).unwrap();
        let actual: Vec<_> = got.iter().map(|a| a.pubkey.to_string()).collect();
        let expected: Vec<_> = c["expected_accounts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k.as_str().unwrap().to_owned())
            .collect();
        assert_eq!(actual, expected);
        assert!(resolve_hook_accounts(
            &hook, &mint, &owner, &mintdata, &meta, &metaowner, &metadata, &keys
        )
        .is_err());
        assert!(invoke(&metadata, &data, &HashMap::new()).is_err());
        assert!(invoke(&metadata, &data[..15], &accounts).is_err());
        for (offset, value) in [
            (86, 255),
            (96, 255),
            (102, 33),
            (88, 31),
            (122, 1),
            (122, 0),
            (123, 255),
            (124, 255),
            (125, 1),
        ] {
            let mut bad = metadata.clone();
            bad[offset] = value;
            assert!(invoke(&bad, &data, &accounts).is_err());
        }
        let mut changed = data.clone();
        changed[8] += 1;
        assert_ne!(invoke(&metadata, &changed, &accounts).unwrap()[2].pubkey, got[2].pubkey);
    }
}
