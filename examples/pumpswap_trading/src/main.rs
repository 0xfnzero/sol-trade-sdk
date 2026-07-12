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
use solana_streamer_sdk::streaming::event_parser::{
    common::filter::EventTypeFilter, protocols::pumpswap::PumpSwapBuyEvent,
};
use solana_streamer_sdk::streaming::event_parser::{
    common::EventType, protocols::pumpswap::PumpSwapSellEvent,
};
use solana_streamer_sdk::streaming::event_parser::{Protocol, UnifiedEvent};
use solana_streamer_sdk::streaming::yellowstone_grpc::{AccountFilter, TransactionFilter};
use solana_streamer_sdk::streaming::YellowstoneGrpc;
use solana_streamer_sdk::{
    match_event, streaming::event_parser::protocols::pumpswap::parser::PUMPSWAP_PROGRAM_ID,
};
use std::str::FromStr;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
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

    let grpc = YellowstoneGrpc::new(
        std::env::var("GRPC_ENDPOINT")
            .unwrap_or_else(|_| "https://solana-yellowstone-grpc.publicnode.com:443".to_string()),
        std::env::var("GRPC_AUTH_TOKEN").ok(),
    )?;

    let callback = create_event_callback(trade_client, blockhash_cache, selection);
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
    let event_type_filter =
        EventTypeFilter { include: vec![EventType::PumpSwapBuy, EventType::PumpSwapSell] };

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
    selection: EventSelection,
) -> impl Fn(Box<dyn UnifiedEvent>) {
    move |event: Box<dyn UnifiedEvent>| {
        match_event!(event, {
            PumpSwapBuyEvent => |e: PumpSwapBuyEvent| {
                let is_wsol = e.base_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT || e.quote_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT;
                let is_usdc = e.base_mint == sol_trade_sdk::constants::USDC_TOKEN_ACCOUNT || e.quote_mint == sol_trade_sdk::constants::USDC_TOKEN_ACCOUNT;
                if !is_wsol && !is_usdc {
                    return;
                }
                if !selection.matches(e.pool, e.base_mint, e.quote_mint, e.metadata.recv_us) {
                    return;
                }
                // Test code, only test one transaction
                if !ALREADY_EXECUTED.swap(true, Ordering::SeqCst) {
                    let event_clone = e.clone();
                    let client = client.clone();
                    let blockhash_cache = blockhash_cache.clone();
                    tokio::spawn(async move {
                        if let Err(err) = pumpswap_trade_with_grpc_buy_event(
                            client,
                            blockhash_cache,
                            event_clone,
                        ).await {
                            eprintln!("Error in trade: {:?}", err);
                            std::process::exit(1);
                        }
                    });
                }
            },
            PumpSwapSellEvent => |e: PumpSwapSellEvent| {
                let is_wsol = e.base_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT || e.quote_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT;
                let is_usdc = e.base_mint == sol_trade_sdk::constants::USDC_TOKEN_ACCOUNT || e.quote_mint == sol_trade_sdk::constants::USDC_TOKEN_ACCOUNT;
                if !is_wsol && !is_usdc {
                    return;
                }
                if !selection.matches(e.pool, e.base_mint, e.quote_mint, e.metadata.recv_us) {
                    return;
                }
                // Test code, only test one transaction
                if !ALREADY_EXECUTED.swap(true, Ordering::SeqCst) {
                    let event_clone = e.clone();
                    let client = client.clone();
                    let blockhash_cache = blockhash_cache.clone();
                    tokio::spawn(async move {
                        if let Err(err) = pumpswap_trade_with_grpc_sell_event(
                            client,
                            blockhash_cache,
                            event_clone,
                        ).await {
                            eprintln!("Error in trade: {:?}", err);
                            std::process::exit(1);
                        }
                    });
                }
            }
        });
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
    trade_info: PumpSwapBuyEvent,
) -> AnyResult<()> {
    let params = PumpSwapParams::from_trade_with_fee_basis_points(
        trade_info.pool,
        trade_info.base_mint,
        trade_info.quote_mint,
        trade_info.pool_base_token_account,
        trade_info.pool_quote_token_account,
        trade_info.pool_base_token_reserves,
        trade_info.pool_quote_token_reserves,
        trade_info.coin_creator_vault_ata,
        trade_info.coin_creator_vault_authority,
        trade_info.base_token_program,
        trade_info.quote_token_program,
        trade_info.protocol_fee_recipient,
        Pubkey::default(),
        trade_info.coin_creator,
        false,
        0,
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
    pumpswap_trade_with_grpc(&client, &blockhash_cache, trade_info.metadata.recv_us, mint, params)
        .await?;
    Ok(())
}

async fn pumpswap_trade_with_grpc_sell_event(
    client: Arc<SolanaTrade>,
    blockhash_cache: BlockhashCache,
    trade_info: PumpSwapSellEvent,
) -> AnyResult<()> {
    let params = PumpSwapParams::from_trade_with_fee_basis_points(
        trade_info.pool,
        trade_info.base_mint,
        trade_info.quote_mint,
        trade_info.pool_base_token_account,
        trade_info.pool_quote_token_account,
        trade_info.pool_base_token_reserves,
        trade_info.pool_quote_token_reserves,
        trade_info.coin_creator_vault_ata,
        trade_info.coin_creator_vault_authority,
        trade_info.base_token_program,
        trade_info.quote_token_program,
        trade_info.protocol_fee_recipient,
        Pubkey::default(),
        trade_info.coin_creator,
        false,
        0,
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
    pumpswap_trade_with_grpc(&client, &blockhash_cache, trade_info.metadata.recv_us, mint, params)
        .await?;
    Ok(())
}

async fn pumpswap_trade_with_grpc(
    client: &SolanaTrade,
    blockhash_cache: &BlockhashCache,
    grpc_recv_us: i64,
    mint_pubkey: Pubkey,
    params: PumpSwapParams,
) -> AnyResult<()> {
    println!("Testing PumpSwap trading...");
    let slippage_basis_points = Some(500);
    let recent_blockhash = blockhash_cache.latest()?;

    let gas_fee_strategy = sol_trade_sdk::common::GasFeeStrategy::new();
    gas_fee_strategy.set_global_fee_strategy(150000, 150000, 500000, 500000, 0.001, 0.001);

    let is_sol = params.base_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT
        || params.quote_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT;
    let program_id = if params.base_mint == mint_pubkey {
        params.base_token_program
    } else if params.quote_mint == mint_pubkey {
        params.quote_token_program
    } else {
        anyhow::bail!("target mint {} does not belong to pool {}", mint_pubkey, params.pool);
    };
    let balance_before =
        client.get_payer_token_balance_with_program(&mint_pubkey, &program_id).await?;

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

    // Exit program
    std::process::exit(0);
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
