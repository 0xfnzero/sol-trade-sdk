use sol_trade_sdk::common::{clock::now_micros, SolanaRpcClient, TradeConfig};
use sol_trade_sdk::TradeTokenType;
use sol_trade_sdk::{
    common::AnyResult,
    swqos::SwqosConfig,
    trading::{
        core::params::{DexParamEnum, PumpSwapParams},
        factory::DexType,
    },
    AccountPolicy, BuyAmount, SellAmount, SimpleBuyParams, SimpleSellParams, SolanaTrade,
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{hash::Hash, pubkey::Pubkey};
use solana_streamer_sdk::streaming::event_parser::protocols::pumpswap::parser::PUMPSWAP_PROGRAM_ID;
use solana_streamer_sdk::streaming::event_parser::{
    common::filter::EventTypeFilter, protocols::pumpswap::PumpSwapBuyEvent,
};
use solana_streamer_sdk::streaming::event_parser::{
    common::EventType, protocols::pumpswap::PumpSwapSellEvent,
};
use solana_streamer_sdk::streaming::event_parser::{DexEvent, Protocol};
use solana_streamer_sdk::streaming::yellowstone_grpc::{AccountFilter, TransactionFilter};
use solana_streamer_sdk::streaming::YellowstoneGrpc;
use std::str::FromStr;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, RwLock,
};
use std::time::{Duration, Instant};
use tokio::sync::watch;

// Global static flag to ensure transaction is executed only once
static ALREADY_EXECUTED: AtomicBool = AtomicBool::new(false);

const BLOCKHASH_REFRESH_INTERVAL: Duration = Duration::from_millis(400);
const MAX_BLOCKHASH_AGE: Duration = Duration::from_secs(20);

#[derive(Clone, Copy)]
struct EventSelection {
    target_mint: Option<Pubkey>,
    target_pool: Option<Pubkey>,
    max_event_age_ms: u64,
}

impl EventSelection {
    fn from_env() -> AnyResult<Self> {
        let target_mint = parse_optional_pubkey("TARGET_MINT")?;
        let target_pool = parse_optional_pubkey("TARGET_POOL")?;
        if target_mint.is_none() && target_pool.is_none() {
            anyhow::bail!("set TARGET_MINT or TARGET_POOL before running this live example");
        }
        let max_event_age_ms = std::env::var("MAX_EVENT_AGE_MS")
            .unwrap_or_else(|_| "1000".to_string())
            .parse::<u64>()
            .map_err(|_| anyhow::anyhow!("MAX_EVENT_AGE_MS must be a positive integer"))?;
        if max_event_age_ms == 0 || max_event_age_ms > i64::MAX as u64 / 1_000 {
            anyhow::bail!("MAX_EVENT_AGE_MS is outside the supported range");
        }
        Ok(Self { target_mint, target_pool, max_event_age_ms })
    }

    fn matches(self, pool: Pubkey, base_mint: Pubkey, quote_mint: Pubkey, recv_us: i64) -> bool {
        if self.target_pool.is_some_and(|target| target != pool) {
            return false;
        }
        if self.target_mint.is_some_and(|target| target != base_mint && target != quote_mint) {
            return false;
        }
        is_event_fresh(recv_us, now_micros(), self.max_event_age_ms)
    }
}

fn parse_optional_pubkey(key: &str) -> AnyResult<Option<Pubkey>> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| Pubkey::from_str(&value).map_err(anyhow::Error::from))
        .transpose()
}

fn is_event_fresh(recv_us: i64, now_us: i64, max_age_ms: u64) -> bool {
    recv_us > 0
        && now_us >= recv_us
        && now_us.saturating_sub(recv_us) <= (max_age_ms as i64).saturating_mul(1_000)
}

#[derive(Clone)]
struct CachedBlockhash {
    hash: Hash,
    fetched_at: Instant,
}

#[derive(Clone, Copy)]
struct PositionBaseline {
    mint: Pubkey,
    token_program: Pubkey,
    amount: u64,
}

enum EventAction {
    BaselineWarmed,
    TradeCompleted,
}

#[derive(Clone)]
struct BlockhashCache {
    receiver: watch::Receiver<CachedBlockhash>,
}

impl BlockhashCache {
    async fn start(rpc: Arc<SolanaRpcClient>) -> AnyResult<Self> {
        let initial =
            CachedBlockhash { hash: rpc.get_latest_blockhash().await?, fetched_at: Instant::now() };
        let (sender, receiver) = watch::channel(initial);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(BLOCKHASH_REFRESH_INTERVAL);
            interval.tick().await;
            loop {
                interval.tick().await;
                match rpc.get_latest_blockhash().await {
                    Ok(hash) => {
                        if sender
                            .send(CachedBlockhash { hash, fetched_at: Instant::now() })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(err) => eprintln!("warning: blockhash refresh failed: {err}"),
                }
            }
        });
        Ok(Self { receiver })
    }

    fn latest(&self) -> AnyResult<Hash> {
        let cached = self.receiver.borrow().clone();
        if cached.fetched_at.elapsed() > MAX_BLOCKHASH_AGE {
            anyhow::bail!("cached blockhash is older than {} seconds", MAX_BLOCKHASH_AGE.as_secs());
        }
        Ok(cached.hash)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Subscribing to GRPC events...");

    let selection = EventSelection::from_env()?;

    let trade_client = Arc::new(create_solana_trade_client().await?);
    let blockhash_cache = BlockhashCache::start(trade_client.infrastructure.rpc.clone()).await?;
    let position_baseline = Arc::new(RwLock::new(None));

    let grpc = YellowstoneGrpc::new(
        std::env::var("GRPC_ENDPOINT")
            .unwrap_or_else(|_| "https://solana-yellowstone-grpc.publicnode.com:443".to_string()),
        std::env::var("GRPC_AUTH_TOKEN").ok(),
    )?;

    let callback =
        create_event_callback(trade_client, blockhash_cache, position_baseline, selection);
    let protocols = vec![Protocol::PumpSwap];
    // Filter accounts
    let account_include = vec![
        PUMPSWAP_PROGRAM_ID.to_string(), // Listen to PumpSwap program ID
    ];
    let account_exclude = vec![];

    let account_required = vec![];

    // Listen to transaction data
    let transaction_filter = TransactionFilter {
        account_include: account_include.clone(),
        account_exclude,
        account_required,
    };

    // Listen to account data belonging to owner programs -> account event monitoring
    let account_filter = AccountFilter { account: vec![], owner: vec![], filters: vec![] };

    // listen to specific event type
    let event_type_filter = EventTypeFilter {
        include: vec![EventType::PumpSwapBuy, EventType::PumpSwapSell],
        ..Default::default()
    };

    grpc.subscribe_events_immediate(
        protocols,
        None,
        vec![transaction_filter],
        vec![account_filter],
        Some(event_type_filter),
        None,
        callback,
    )
    .await?;

    tokio::signal::ctrl_c().await?;

    Ok(())
}

/// Create an event callback function that handles different types of events
fn create_event_callback(
    client: Arc<SolanaTrade>,
    blockhash_cache: BlockhashCache,
    position_baseline: Arc<RwLock<Option<PositionBaseline>>>,
    selection: EventSelection,
) -> impl Fn(DexEvent) {
    move |event: DexEvent| match event {
        DexEvent::PumpSwapBuyEvent(e) => {
            let is_wsol = e.base_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT
                || e.quote_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT;
            let is_usdc = e.base_mint == sol_trade_sdk::constants::USDC_TOKEN_ACCOUNT
                || e.quote_mint == sol_trade_sdk::constants::USDC_TOKEN_ACCOUNT;
            if !is_wsol && !is_usdc {
                return;
            }
            if !selection.matches(e.pool, e.base_mint, e.quote_mint, e.metadata.recv_us) {
                return;
            }
            // Test code, only test one transaction
            if ALREADY_EXECUTED
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                let client = client.clone();
                let blockhash_cache = blockhash_cache.clone();
                let position_baseline = position_baseline.clone();
                let was_preparing =
                    position_baseline.read().map(|baseline| baseline.is_none()).unwrap_or(true);
                tokio::spawn(async move {
                    match pumpswap_trade_with_grpc_buy_event(
                        client,
                        blockhash_cache,
                        position_baseline,
                        selection,
                        e,
                    )
                    .await
                    {
                        Ok(EventAction::BaselineWarmed) => {
                            ALREADY_EXECUTED.store(false, Ordering::Release);
                        }
                        Ok(EventAction::TradeCompleted) => {}
                        Err(err) if was_preparing => {
                            eprintln!("baseline warmup failed; waiting for a later event: {err:?}");
                            ALREADY_EXECUTED.store(false, Ordering::Release);
                        }
                        Err(err) => {
                            eprintln!(
                                "trade failed after entering execution state: {err:?}; automatic retry is disabled because submission status or position state may be uncertain"
                            );
                        }
                    }
                });
            }
        }
        DexEvent::PumpSwapSellEvent(e) => {
            let is_wsol = e.base_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT
                || e.quote_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT;
            let is_usdc = e.base_mint == sol_trade_sdk::constants::USDC_TOKEN_ACCOUNT
                || e.quote_mint == sol_trade_sdk::constants::USDC_TOKEN_ACCOUNT;
            if !is_wsol && !is_usdc {
                return;
            }
            if !selection.matches(e.pool, e.base_mint, e.quote_mint, e.metadata.recv_us) {
                return;
            }
            // Test code, only test one transaction
            if ALREADY_EXECUTED
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                let client = client.clone();
                let blockhash_cache = blockhash_cache.clone();
                let position_baseline = position_baseline.clone();
                let was_preparing =
                    position_baseline.read().map(|baseline| baseline.is_none()).unwrap_or(true);
                tokio::spawn(async move {
                    match pumpswap_trade_with_grpc_sell_event(
                        client,
                        blockhash_cache,
                        position_baseline,
                        selection,
                        e,
                    )
                    .await
                    {
                        Ok(EventAction::BaselineWarmed) => {
                            ALREADY_EXECUTED.store(false, Ordering::Release);
                        }
                        Ok(EventAction::TradeCompleted) => {}
                        Err(err) if was_preparing => {
                            eprintln!("baseline warmup failed; waiting for a later event: {err:?}");
                            ALREADY_EXECUTED.store(false, Ordering::Release);
                        }
                        Err(err) => {
                            eprintln!(
                                "trade failed after entering execution state: {err:?}; automatic retry is disabled because submission status or position state may be uncertain"
                            );
                        }
                    }
                });
            }
        }
        _ => {}
    }
}

/// Create SolanaTrade client
/// Initializes a new SolanaTrade client with configuration
async fn create_solana_trade_client() -> AnyResult<SolanaTrade> {
    println!("Initializing SolanaTrade client...");
    let payer = sol_trade_sdk::common::keypair::load_keypair_from_env("PRIVATE_KEY")?;
    let rpc_url = std::env::var("RPC_URL")
        .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string());
    let commitment = CommitmentConfig::confirmed();
    let swqos_configs: Vec<SwqosConfig> = vec![SwqosConfig::Default(rpc_url.clone())];
    let trade_config = TradeConfig::builder(rpc_url, swqos_configs, commitment)
        // .create_wsol_ata_on_startup(true)  // default: true
        // .use_seed_optimize(true)            // default: true
        // .log_enabled(true)                  // default: true
        // .check_min_tip(false)               // default: false
        // .swqos_cores_from_end(false)        // default: false
        // .mev_protection(false)              // default: false
        .build();
    let solana_trade = SolanaTrade::new(Arc::new(payer), trade_config).await;
    println!("SolanaTrade client initialized successfully");
    Ok(solana_trade)
}

async fn pumpswap_trade_with_grpc_buy_event(
    client: Arc<SolanaTrade>,
    blockhash_cache: BlockhashCache,
    position_baseline: Arc<RwLock<Option<PositionBaseline>>>,
    selection: EventSelection,
    trade_info: PumpSwapBuyEvent,
) -> AnyResult<EventAction> {
    let params = PumpSwapParams::from_trade_with_fee_basis_points(
        trade_info.pool,
        trade_info.base_mint,
        trade_info.quote_mint,
        trade_info.pool_base_token_account,
        trade_info.pool_quote_token_account,
        trade_info.pool_base_token_reserves,
        trade_info.pool_quote_token_reserves,
        trade_info.virtual_quote_reserves,
        trade_info.coin_creator_vault_ata,
        trade_info.coin_creator_vault_authority,
        trade_info.base_token_program,
        trade_info.quote_token_program,
        trade_info.protocol_fee_recipient,
        Pubkey::default(),
        trade_info.coin_creator,
        trade_info.cashback_fee_basis_points != 0 || trade_info.cashback != 0,
        trade_info.cashback_fee_basis_points,
        trade_info.lp_fee_basis_points,
        trade_info.protocol_fee_basis_points,
        trade_info.coin_creator_fee_basis_points,
    );
    let mint = if trade_info.base_mint == sol_trade_sdk::constants::USDC_TOKEN_ACCOUNT
        || trade_info.base_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT
    {
        trade_info.quote_mint
    } else {
        trade_info.base_mint
    };
    pumpswap_trade_with_grpc(
        &client,
        &blockhash_cache,
        &position_baseline,
        trade_info.metadata.recv_us,
        selection.max_event_age_ms,
        mint,
        params,
    )
    .await
}

async fn pumpswap_trade_with_grpc_sell_event(
    client: Arc<SolanaTrade>,
    blockhash_cache: BlockhashCache,
    position_baseline: Arc<RwLock<Option<PositionBaseline>>>,
    selection: EventSelection,
    trade_info: PumpSwapSellEvent,
) -> AnyResult<EventAction> {
    let params = PumpSwapParams::from_trade_with_fee_basis_points(
        trade_info.pool,
        trade_info.base_mint,
        trade_info.quote_mint,
        trade_info.pool_base_token_account,
        trade_info.pool_quote_token_account,
        trade_info.pool_base_token_reserves,
        trade_info.pool_quote_token_reserves,
        trade_info.virtual_quote_reserves,
        trade_info.coin_creator_vault_ata,
        trade_info.coin_creator_vault_authority,
        trade_info.base_token_program,
        trade_info.quote_token_program,
        trade_info.protocol_fee_recipient,
        Pubkey::default(),
        trade_info.coin_creator,
        trade_info.cashback_fee_basis_points != 0 || trade_info.cashback != 0,
        trade_info.cashback_fee_basis_points,
        trade_info.lp_fee_basis_points,
        trade_info.protocol_fee_basis_points,
        trade_info.coin_creator_fee_basis_points,
    );
    let mint = if trade_info.base_mint == sol_trade_sdk::constants::USDC_TOKEN_ACCOUNT
        || trade_info.base_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT
    {
        trade_info.quote_mint
    } else {
        trade_info.base_mint
    };
    pumpswap_trade_with_grpc(
        &client,
        &blockhash_cache,
        &position_baseline,
        trade_info.metadata.recv_us,
        selection.max_event_age_ms,
        mint,
        params,
    )
    .await
}

async fn pumpswap_trade_with_grpc(
    client: &SolanaTrade,
    blockhash_cache: &BlockhashCache,
    position_baseline: &Arc<RwLock<Option<PositionBaseline>>>,
    grpc_recv_us: i64,
    max_event_age_ms: u64,
    mint_pubkey: Pubkey,
    params: PumpSwapParams,
) -> AnyResult<EventAction> {
    println!("Testing PumpSwap trading...");
    validate_pumpswap_snapshot(&params)?;
    if !is_event_fresh(grpc_recv_us, now_micros(), max_event_age_ms) {
        anyhow::bail!("event became stale before transaction construction");
    }
    let slippage_basis_points = Some(500);

    let is_sol = params.base_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT
        || params.quote_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT;
    let program_id = if params.base_mint == mint_pubkey {
        params.base_token_program
    } else if params.quote_mint == mint_pubkey {
        params.quote_token_program
    } else {
        anyhow::bail!("target mint {} does not belong to pool {}", mint_pubkey, params.pool);
    };
    let baseline = position_baseline
        .read()
        .map_err(|_| anyhow::anyhow!("position baseline lock is poisoned"))?
        .as_ref()
        .copied();
    let balance_before = if let Some(baseline) = baseline {
        if baseline.mint != mint_pubkey || baseline.token_program != program_id {
            anyhow::bail!("cached position baseline belongs to a different mint or token program");
        }
        baseline.amount
    } else {
        let amount = client.get_payer_token_balance_with_program(&mint_pubkey, &program_id).await?;
        let mut baseline = position_baseline
            .write()
            .map_err(|_| anyhow::anyhow!("position baseline lock is poisoned"))?;
        *baseline = Some(PositionBaseline { mint: mint_pubkey, token_program: program_id, amount });
        println!(
            "Position baseline warmed at {} base units; waiting for the next fresh matching event",
            amount
        );
        return Ok(EventAction::BaselineWarmed);
    };

    let recent_blockhash = blockhash_cache.latest()?;
    let gas_fee_strategy = sol_trade_sdk::common::GasFeeStrategy::new();
    gas_fee_strategy.set_global_fee_strategy(150000, 150000, 500000, 500000, 0.001, 0.001);
    if !is_event_fresh(grpc_recv_us, now_micros(), max_event_age_ms) {
        anyhow::bail!("event became stale while preparing the transaction");
    }

    // Buy tokens
    println!("Buying tokens from PumpSwap...");
    let buy_token_amount = 300_000;
    let buy_params = SimpleBuyParams::new(
        DexType::PumpSwap,
        if is_sol { TradeTokenType::SOL } else { TradeTokenType::USDC },
        mint_pubkey,
        BuyAmount::WithMaxInput { quote_amount: buy_token_amount },
        DexParamEnum::PumpSwap(params.clone()),
        recent_blockhash,
        gas_fee_strategy.clone(),
    )
    .slippage_basis_points(slippage_basis_points.unwrap_or(500))
    .account_policy(AccountPolicy::Auto)
    .wait_tx_confirmed(true)
    .grpc_recv_us(grpc_recv_us);
    let (ok, sigs, err, _) = client.buy_simple(buy_params).await?;
    if !ok {
        anyhow::bail!("buy failed: {:?}; signatures: {:?}", err, sigs);
    }

    // Sell tokens
    println!("Selling tokens from PumpSwap...");

    let balance_after =
        client.get_payer_token_balance_with_program(&mint_pubkey, &program_id).await?;
    let position_amount = balance_after.checked_sub(balance_before).ok_or_else(|| {
        anyhow::anyhow!(
            "token balance decreased from {} to {}; refusing to sell existing holdings",
            balance_before,
            balance_after
        )
    })?;
    if position_amount == 0 {
        anyhow::bail!("confirmed buy did not increase token balance; refusing to sell");
    }
    let sell_params_from_rpc =
        PumpSwapParams::from_pool_address_by_rpc(&client.infrastructure.rpc, &params.pool).await?;
    let sell_params = SimpleSellParams::new(
        DexType::PumpSwap,
        if is_sol { TradeTokenType::SOL } else { TradeTokenType::USDC },
        mint_pubkey,
        SellAmount::ExactInput(position_amount),
        DexParamEnum::PumpSwap(sell_params_from_rpc),
        blockhash_cache.latest()?,
        gas_fee_strategy,
    )
    .slippage_basis_points(slippage_basis_points.unwrap_or(500))
    .account_policy(AccountPolicy::Auto)
    .wait_tx_confirmed(true)
    .with_tip(false);
    let (ok, sigs, err, _) = client.sell_simple(sell_params).await?;
    if !ok {
        anyhow::bail!("sell failed: {:?}; signatures: {:?}", err, sigs);
    }

    println!("Round-trip example completed; further matching events remain locked out");
    Ok(EventAction::TradeCompleted)
}

fn validate_pumpswap_snapshot(params: &PumpSwapParams) -> AnyResult<()> {
    let required = [
        ("pool", params.pool),
        ("base_mint", params.base_mint),
        ("quote_mint", params.quote_mint),
        ("pool_base_token_account", params.pool_base_token_account),
        ("pool_quote_token_account", params.pool_quote_token_account),
        ("coin_creator_vault_ata", params.coin_creator_vault_ata),
        ("coin_creator_vault_authority", params.coin_creator_vault_authority),
        ("base_token_program", params.base_token_program),
        ("quote_token_program", params.quote_token_program),
    ];
    for (name, value) in required {
        if value == Pubkey::default() {
            anyhow::bail!("event snapshot is missing {name}");
        }
    }
    if params.pool_base_token_reserves == 0 || params.pool_quote_token_reserves == 0 {
        anyhow::bail!("event snapshot has an empty raw pool reserve");
    }
    params.effective_quote_reserves()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_event_fresh;

    #[test]
    fn event_freshness_has_a_strict_boundary() {
        assert!(!is_event_fresh(0, 1_000_000, 100));
        assert!(!is_event_fresh(1_000_001, 1_000_000, 100));
        assert!(!is_event_fresh(899_999, 1_000_000, 100));
        assert!(is_event_fresh(900_000, 1_000_000, 100));
    }
}
