//! Pre-buy risk gate fed by a hosted audit API (Dexra).
//!
//! A background task polls Dexra's launch feed, which already carries a
//! verdict per launch (`ok`, `caution`, `avoid`, `unknown`) built from mint and
//! freeze authorities, liquidity and its lock, the creator's earlier launches
//! and the first buyers' funding graph. Each poll publishes an immutable
//! snapshot; `TradeRiskGate::check_buy` only reads it, so the buy path never
//! waits on HTTP. Buys are allowed only for mints rated `ok` in a fresh
//! snapshot: `caution`, `avoid`, `unknown`, missing and stale all reject.

use std::{
    collections::HashMap,
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{bail, Context};
use arc_swap::ArcSwap;
use serde::Deserialize;
use sol_trade_sdk::{TradeBuyParams, TradeRiskGate};
use solana_sdk::pubkey::Pubkey;

const DEFAULT_URL: &str = "https://api.dexra.dev";
const DEFAULT_POLL_SECS: u64 = 60;
const DEFAULT_PLATFORMS: &str = "pump_fun,pump_swap,letsbonk";
// A snapshot older than this many poll intervals means the refresher is failing.
const STALE_AFTER_POLLS: u32 = 3;

#[derive(Deserialize)]
struct Envelope {
    result: Feed,
}

#[derive(Deserialize)]
struct Feed {
    launches: Vec<Launch>,
}

#[derive(Deserialize)]
struct Launch {
    mint: String,
    platform: String,
    verdict: Verdict,
    reasons: Vec<String>,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(rename_all = "lowercase")]
enum Verdict {
    Ok,
    Unknown,
    Caution,
    Avoid,
}

enum Decision {
    Allow,
    Deny(String),
}

#[derive(Default)]
struct RiskSnapshot {
    fetched_at: Option<Instant>,
    decisions: HashMap<Pubkey, Decision>,
}

struct DexraRiskGate {
    snapshot: Arc<ArcSwap<RiskSnapshot>>,
    max_age: Duration,
}

impl DexraRiskGate {
    fn check_mint(&self, mint: &Pubkey) -> anyhow::Result<()> {
        let snapshot = self.snapshot.load();
        match snapshot.fetched_at {
            Some(at) if at.elapsed() <= self.max_age => {}
            _ => bail!("Dexra verdicts are missing or stale; not buying {mint}"),
        }
        match snapshot.decisions.get(mint) {
            Some(Decision::Allow) => Ok(()),
            Some(Decision::Deny(why)) => bail!("Dexra rejected {mint}: {why}"),
            None => bail!("no Dexra verdict for {mint} yet"),
        }
    }
}

impl TradeRiskGate for DexraRiskGate {
    fn check_buy(&self, params: &TradeBuyParams) -> anyhow::Result<()> {
        self.check_mint(&params.mint)
    }
}

struct Refresher {
    http: reqwest::Client,
    url: String,
    api_key: String,
    platforms: String,
    limit: u32,
}

impl Refresher {
    async fn fetch(&self) -> anyhow::Result<Vec<Launch>> {
        let response = self
            .http
            .get(format!("{}/new_launches", self.url))
            .bearer_auth(&self.api_key)
            .query(&[("minutes", "15"), ("platforms", self.platforms.as_str())])
            .query(&[("limit", self.limit)])
            .send()
            .await?;
        match response.status().as_u16() {
            200 => Ok(response.json::<Envelope>().await?.result.launches),
            401 => bail!("DEXRA_API_KEY is not a Dexra key, or it was revoked"),
            402 => {
                bail!("payment required: today's credits are spent or the plan does not cover this feed")
            }
            429 => bail!("rate limited; raise DEXRA_POLL_SECS"),
            status => bail!("HTTP {status}: {}", response.text().await.unwrap_or_default()),
        }
    }

    /// One mint can have several pools (a pump.fun curve and its PumpSwap pool);
    /// the worst verdict among them decides.
    fn snapshot(launches: &[Launch]) -> RiskSnapshot {
        let mut worst: HashMap<Pubkey, &Launch> = HashMap::new();
        for launch in launches {
            let Ok(mint) = Pubkey::from_str(&launch.mint) else { continue };
            let entry = worst.entry(mint).or_insert(launch);
            if launch.verdict > entry.verdict {
                *entry = launch;
            }
        }
        let decisions = worst
            .into_iter()
            .map(|(mint, launch)| {
                let decision = match launch.verdict {
                    Verdict::Ok => Decision::Allow,
                    verdict => Decision::Deny(format!(
                        "{verdict:?} on {}: {}",
                        launch.platform,
                        launch.reasons.first().map(String::as_str).unwrap_or("no reason given")
                    )),
                };
                (mint, decision)
            })
            .collect();
        RiskSnapshot { fetched_at: Some(Instant::now()), decisions }
    }

    async fn run(self, snapshot: Arc<ArcSwap<RiskSnapshot>>, every: Duration) {
        let mut ticker = tokio::time::interval(every);
        loop {
            ticker.tick().await;
            match self.fetch().await {
                Ok(launches) => {
                    for launch in &launches {
                        println!(
                            "{:<8} {:<10} {}  {}",
                            format!("{:?}", launch.verdict).to_lowercase(),
                            launch.platform,
                            launch.mint,
                            launch.reasons.first().map(String::as_str).unwrap_or("")
                        );
                    }
                    snapshot.store(Arc::new(Self::snapshot(&launches)));
                }
                // Keep the previous snapshot; the gate rejects it once it is stale.
                Err(error) => eprintln!("Dexra refresh failed: {error:#}"),
            }
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let api_key = std::env::var("DEXRA_API_KEY")
        .context("set DEXRA_API_KEY (create a free key at https://dexra.dev/app/keys)")?;
    let poll = Duration::from_secs(
        std::env::var("DEXRA_POLL_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_POLL_SECS),
    );
    let refresher = Refresher {
        http: reqwest::Client::builder().timeout(Duration::from_secs(30)).build()?,
        url: std::env::var("DEXRA_URL").unwrap_or_else(|_| DEFAULT_URL.to_string()),
        api_key,
        platforms: std::env::var("DEXRA_PLATFORMS")
            .unwrap_or_else(|_| DEFAULT_PLATFORMS.to_string()),
        limit: std::env::var("DEXRA_LIMIT").ok().and_then(|s| s.parse().ok()).unwrap_or(20),
    };

    let snapshot = Arc::new(ArcSwap::from_pointee(RiskSnapshot::default()));
    let gate =
        Arc::new(DexraRiskGate { snapshot: snapshot.clone(), max_age: poll * STALE_AFTER_POLLS });
    tokio::spawn(refresher.run(snapshot, poll));

    // In a bot, attach the gate once and buy as usual; rejected mints return an
    // error from `buy` / `buy_simple` before any transaction is built:
    //
    //     let client = SolanaTrade::new(Arc::new(payer), trade_config).await
    //         .with_risk_gate(gate.clone());
    //
    // Here the mints given on the command line are checked against the gate.
    let mints = std::env::args()
        .skip(1)
        .map(|arg| Pubkey::from_str(&arg).with_context(|| format!("not a mint: {arg}")))
        .collect::<anyhow::Result<Vec<_>>>()?;
    loop {
        tokio::time::sleep(poll).await;
        for mint in &mints {
            match gate.check_mint(mint) {
                Ok(()) => println!("gate: buy {mint}"),
                Err(error) => println!("gate: {error}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURVE_OK_POOL_AVOID: &str = r#"{"result":{"launches":[
        {"mint":"So11111111111111111111111111111111111111112","platform":"pump_fun","verdict":"ok","reasons":[]},
        {"mint":"So11111111111111111111111111111111111111112","platform":"pump_swap","verdict":"avoid","reasons":["liquidity withdrawn"]},
        {"mint":"EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v","platform":"pump_fun","verdict":"ok","reasons":[]},
        {"mint":"Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB","platform":"pump_fun","verdict":"unknown","reasons":["first buyers not analyzed yet"]}
    ]}}"#;

    fn gate(snapshot: RiskSnapshot) -> DexraRiskGate {
        DexraRiskGate {
            snapshot: Arc::new(ArcSwap::from_pointee(snapshot)),
            max_age: Duration::from_secs(60),
        }
    }

    fn mint(s: &str) -> Pubkey {
        Pubkey::from_str(s).unwrap()
    }

    #[test]
    fn allows_only_ok_and_worst_pool_decides() {
        let feed: Envelope = serde_json::from_str(CURVE_OK_POOL_AVOID).unwrap();
        let gate = gate(Refresher::snapshot(&feed.result.launches));
        assert!(gate.check_mint(&mint("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")).is_ok());
        let avoid = gate.check_mint(&mint("So11111111111111111111111111111111111111112"));
        assert!(avoid.unwrap_err().to_string().contains("liquidity withdrawn"));
        assert!(gate.check_mint(&mint("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB")).is_err());
        assert!(gate.check_mint(&Pubkey::new_unique()).is_err());
    }

    #[test]
    fn rejects_before_first_refresh() {
        let gate = gate(RiskSnapshot::default());
        assert!(gate.check_mint(&Pubkey::new_unique()).is_err());
    }
}
