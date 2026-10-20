//! Coherent offline account snapshots for quote and depth tools.
use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::Value;
use sol_trade_sdk::trading::core::params::{
    CacheReadContext, CachedAccount, SubscriptionAccountCache,
};
use solana_sdk::pubkey::Pubkey;

pub struct AccountSnapshot {
    pub cache: SubscriptionAccountCache,
    pub context: CacheReadContext,
    pub unix_timestamp: u64,
}

impl AccountSnapshot {
    pub fn from_document(document: &Value) -> Result<Self> {
        let slot = document["slot"].as_u64().context("snapshot slot missing")?;
        let mut cache = SubscriptionAccountCache::default();
        let mut clock = None;
        for (address, account) in document["accounts"].as_object().context("accounts missing")? {
            if account.is_null() {
                continue;
            }
            let key: Pubkey = address.parse().context("invalid account address")?;
            let owner: Pubkey = account["owner"].as_str().context("owner missing")?.parse()?;
            ensure!(account["data"][1] == "base64", "snapshot account data must be base64");
            let data =
                STANDARD.decode(account["data"][0].as_str().context("account data missing")?)?;
            if key == solana_sdk::sysvar::clock::ID {
                ensure!(
                    owner == solana_sdk::pubkey!("Sysvar1111111111111111111111111111111111111"),
                    "invalid Clock owner"
                );
                ensure!(data.len() >= 40, "Clock truncated");
                ensure!(
                    u64::from_le_bytes(data[..8].try_into()?) == slot,
                    "Clock does not match the snapshot slot"
                );
                clock = Some((
                    u64::from_le_bytes(data[16..24].try_into()?),
                    u64::try_from(i64::from_le_bytes(data[32..40].try_into()?))?,
                ));
            }
            cache.update(key, CachedAccount { owner, data, slot, write_version: 0 })?;
        }
        let (epoch, unix_timestamp) = clock.context("Clock missing")?;
        Ok(Self {
            cache,
            context: CacheReadContext { slot, epoch, maximum_slot_age: 0 },
            unix_timestamp,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_mismatched_clock_and_owner_before_quoting() {
        let mut data = vec![0u8; 40];
        data[..8].copy_from_slice(&100u64.to_le_bytes());
        data[16..24].copy_from_slice(&3u64.to_le_bytes());
        data[32..40].copy_from_slice(&1_700_000_000i64.to_le_bytes());
        let mut document = serde_json::json!({
            "slot":100, "accounts":{
                "SysvarC1ock11111111111111111111111111111111":{
                    "owner":"Sysvar1111111111111111111111111111111111111",
                    "data":[STANDARD.encode(data), "base64"]
                }
            }
        });
        assert_eq!(AccountSnapshot::from_document(&document).unwrap().context.epoch, 3);
        document["slot"] = 101.into();
        assert!(AccountSnapshot::from_document(&document).is_err());
        document["slot"] = 100.into();
        document["accounts"]["SysvarC1ock11111111111111111111111111111111"]["owner"] =
            Pubkey::new_unique().to_string().into();
        assert!(AccountSnapshot::from_document(&document).is_err());
    }
}
