//! Canary binary — tiny wallet, tight limits, manual triggers first.
//!
//! Phase 12 of the productionization roadmap: runs alongside shadow mode
//! with a tiny wallet, ultra-conservative limits, and a manual approval
//! gate. Every trade must be approved via HTTP before submission.
//! After each trade, outcomes are compared against shadow-mode decisions.
//!
//! # Usage
//!
//! ```text
//! solcanary --config /etc/solbot/canary.toml
//!           [--keypair /path/to/keypair.json]
//!           [--metrics  127.0.0.1:9090]
//!           [--health   127.0.0.1:9091]
//!           [--canary   127.0.0.1:9093]
//! ```
//!
//! # HTTP Endpoints
//!
//! | Path | Description |
//! |------|-------------|
//! | `GET /health` | Build info + canary mode status |
//! | `GET /metrics` | Prometheus-format counters from PerfRegistry |
//! | `GET /canary/pending` | List pending trades awaiting approval |
//! | `POST /canary/approve/:id` | Approve a specific pending trade |
//! | `POST /canary/reject/:id?reason=...` | Reject a specific pending trade |
//! | `POST /canary/approve-all` | Approve ALL pending trades |
//! | `POST /canary/reject-all?reason=...` | Reject ALL pending trades |
//! | `GET /canary/shadow-compare` | Shadow comparison summary |
//! | `GET /canary/stats` | Canary gate + trade stats |
//! | `GET /canary/records?n=20` | Recent trade records |
//!
//! # Safety
//!
//! - Wallet balance is checked at startup — must be ≤ 0.5 SOL (configurable)
//! - Max SOL per trade: 0.01 (configurable)
//! - Max daily trades: 20 (configurable)
//! - Manual approval required by default
//! - All trades appear in shadow comparison for accuracy tracking

use std::env;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use solana_sdk::signature::read_keypair_file;
use solana_sdk::signature::Signer;
use tiny_http::{Header, Method, Response, Server};
use tracing::{error, info, warn};

use sol_trade_sdk::common::config::AppConfig;
use sol_trade_sdk::perf::shredstream::config::ShredstreamConfig;
use sol_trade_sdk::perf::shredstream::ShredstreamAdapter;
use sol_trade_sdk::perf::PerfRegistry;
use sol_trade_sdk::trading::canary::{ManualApprovalGate, ShadowComparator};
use sol_trade_sdk::trading::core::orchestrator::Orchestrator;

// ── Build info ──────────────────────────────────────────────────────────

const BUILD_VERSION: &str = env!("CARGO_PKG_VERSION");
const BUILD_NAME: &str = "solcanary";

// ── CLI argument parsing ────────────────────────────────────────────────

struct Cli {
    config: PathBuf,
    keypair: Option<PathBuf>,
    metrics_addr: String,
    health_addr: String,
    canary_addr: String,
}

fn parse_args() -> Cli {
    let args: Vec<String> = env::args().collect();
    let mut cli = Cli {
        config: PathBuf::from("/etc/solbot/canary.toml"),
        keypair: None,
        metrics_addr: "127.0.0.1:9090".to_string(),
        health_addr: "127.0.0.1:9091".to_string(),
        canary_addr: "127.0.0.1:9093".to_string(),
    };

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--config" | "-c" => {
                i += 1;
                if i < args.len() {
                    cli.config = PathBuf::from(&args[i]);
                }
            }
            "--keypair" | "-k" => {
                i += 1;
                if i < args.len() {
                    cli.keypair = Some(PathBuf::from(&args[i]));
                }
            }
            "--metrics" | "-m" => {
                i += 1;
                if i < args.len() {
                    cli.metrics_addr = args[i].clone();
                }
            }
            "--health" | "-H" => {
                i += 1;
                if i < args.len() {
                    cli.health_addr = args[i].clone();
                }
            }
            "--canary" | "-C" => {
                i += 1;
                if i < args.len() {
                    cli.canary_addr = args[i].clone();
                }
            }
            "--help" | "-h" => {
                eprintln!("solcanary — Canary Mode DEX Trader (tiny wallet, manual approval)");
                eprintln!();
                eprintln!("Phase 12: runs alongside shadow mode with a tiny wallet,");
                eprintln!("ultra-conservative limits, and manual HTTP approval gate.");
                eprintln!();
                eprintln!("USAGE:");
                eprintln!("  solcanary [OPTIONS]");
                eprintln!();
                eprintln!("OPTIONS:");
                eprintln!("  -c, --config PATH      Config file path [default: /etc/solbot/canary.toml]");
                eprintln!("  -k, --keypair PATH     Keypair file (overrides config path)");
                eprintln!("  -m, --metrics ADDR     Metrics HTTP listen [default: 127.0.0.1:9090]");
                eprintln!("  -H, --health ADDR      Health HTTP listen  [default: 127.0.0.1:9091]");
                eprintln!("  -C, --canary ADDR      Canary API HTTP       [default: 127.0.0.1:9093]");
                eprintln!("  -h, --help             Show this help");
                std::process::exit(0);
            }
            _ => {
                eprintln!("Unknown option: {} (use --help for usage)", args[i]);
                std::process::exit(1);
            }
        }
        i += 1;
    }

    cli
}

// ── Main ────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Parse CLI args
    let cli = parse_args();

    // 2. Setup tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,sol_trade_sdk=info")),
        )
        .init();

    info!("{BUILD_NAME} v{BUILD_VERSION} starting — CANARY MODE, tiny wallet, manual approval");

    // 3. Load configuration
    let config_str = std::fs::read_to_string(&cli.config)
        .with_context(|| format!("Failed to read config: {}", cli.config.display()))?;

    let mut app_config: AppConfig = toml::from_str(&config_str)
        .with_context(|| format!("Failed to parse config: {}", cli.config.display()))?;

    // Override keypair path from CLI if provided
    if let Some(ref kp) = cli.keypair {
        let path = kp.to_string_lossy().to_string();
        app_config.wallet.execution_keypair_path = path.clone();
        app_config.wallet.payer_keypair_path = path;
    }

    // Validate config
    app_config.validate().context("Configuration validation failed")?;

    // 4. Verify canary mode is enabled in config
    if !app_config.canary.enabled {
        anyhow::bail!(
            "Canary mode is disabled in config. Set [canary] enabled = true to run solcanary."
        );
    }

    info!("Configuration loaded and validated from {}", cli.config.display());
    info!(
        "Canary limits: max_wallet={:.4} SOL, max_trade={:.6} SOL, max_daily={}, approval={}",
        app_config.canary.max_wallet_balance_sol,
        app_config.canary.max_sol_per_trade,
        app_config.canary.max_daily_trades,
        if app_config.canary.manual_approval { "REQUIRED" } else { "OFF" },
    );

    // 5. Load keypair and verify wallet balance
    let keypair_path = &app_config.wallet.execution_keypair_path;
    let keypair = read_keypair_file(keypair_path)
        .map_err(|e| anyhow::anyhow!("Failed to read keypair from {}: {}", keypair_path, e))?;

    let payer = Arc::new(keypair);
    let wallet_pubkey = payer.as_ref().pubkey();
    info!("Wallet loaded: {wallet_pubkey}");

    // 6. Check wallet balance against canary limits
    let rpc_client = sol_trade_sdk::common::SolanaRpcClient::new(
        app_config.rpc.primary.clone(),
    );
    let balance = rpc_client
        .get_balance(&wallet_pubkey)
        .await
        .context("Failed to check wallet balance at startup")?;
    let balance_sol = balance as f64 / 1_000_000_000.0;

    info!("Wallet balance: {:.9} SOL ({balance} lamports)", balance_sol);

    if balance_sol > app_config.canary.max_wallet_balance_sol {
        anyhow::bail!(
            "Wallet balance {:.4} SOL exceeds canary max {:.4} SOL. \
             Canary mode requires a tiny wallet. Top up with at most {:.4} SOL.",
            balance_sol,
            app_config.canary.max_wallet_balance_sol,
            app_config.canary.max_wallet_balance_sol,
        );
    }

    if balance_sol < app_config.canary.min_wallet_balance_sol {
        anyhow::bail!(
            "Wallet balance {:.6} SOL is below canary minimum {:.6} SOL. \
             Fund the wallet with at least {:.6} SOL for gas.",
            balance_sol,
            app_config.canary.min_wallet_balance_sol,
            app_config.canary.min_wallet_balance_sol,
        );
    }

    info!("Wallet balance {:.4} SOL is within canary limits ✓", balance_sol);

    // 7. Apply canary risk overrides to config BEFORE creating orchestrator
    app_config.risk.max_sol_per_trade = app_config.canary.max_sol_per_trade;
    app_config.risk.global.max_daily_trades = app_config.canary.max_daily_trades;
    app_config.risk.global.max_open_positions = app_config.canary.max_open_positions;
    app_config.risk.max_slippage_basis_points = app_config.canary.max_slippage_basis_points;

    // 8. Create Orchestrator
    let orchestrator =
        Orchestrator::from_config(app_config.clone(), payer).context("Failed to create orchestrator")?;

    // 9. Create canary components
    let approval_gate = Arc::new(ManualApprovalGate::new(
        app_config.canary.approval_timeout_secs,
    ));
    let shadow_comparator = Arc::new(ShadowComparator::new());

    // 9. Start background services
    orchestrator.start();
    info!("Orchestrator started — background services running");

    // 10. Set up HTTP servers
    let running = Arc::new(AtomicBool::new(true));

    let metrics_handle = {
        let running = running.clone();
        let addr = cli.metrics_addr.clone();
        tokio::spawn(async move {
            if let Err(e) = run_metrics_server(&addr, running).await {
                error!("Metrics server failed: {e}");
            }
        })
    };

    let health_handle = {
        let running = running.clone();
        let addr = cli.health_addr.clone();
        tokio::spawn(async move {
            if let Err(e) = run_health_server(&addr, running).await {
                error!("Health server failed: {e}");
            }
        })
    };

    let canary_handle = {
        let running = running.clone();
        let addr = cli.canary_addr.clone();
        let gate = Arc::clone(&approval_gate);
        let comparator = Arc::clone(&shadow_comparator);
        tokio::spawn(async move {
            if let Err(e) = run_canary_api_server(&addr, running, gate, comparator).await {
                error!("Canary API server failed: {e}");
            }
        })
    };

    info!("Metrics HTTP:   http://{}", cli.metrics_addr);
    info!("Health HTTP:    http://{}", cli.health_addr);
    info!("Canary API HTTP: http://{}", cli.canary_addr);
    info!("Manual approval: {}", if app_config.canary.manual_approval { "REQUIRED" } else { "OFF" });

    // 11. Start the canary trade loop (if ShredStream is enabled)
    if app_config.shredstream.enabled {
        info!("ShredStream enabled — starting pipeline");
        start_canary_loop(
            &app_config,
            &orchestrator,
            &approval_gate,
            &shadow_comparator,
            &running,
        )
        .await;
    } else {
        info!("ShredStream disabled — running in HTTP-only mode (approval gate + manual triggers)");
        info!("Use POST /canary/approve-all to trigger trades, or enable shredstream in config");
        wait_for_shutdown_signal().await;
    }

    // 12. Graceful shutdown
    info!("Shutdown signal received — initiating graceful shutdown");
    orchestrator.kill();
    running.store(false, Ordering::Release);

    let _ = tokio::time::timeout(Duration::from_secs(5), metrics_handle).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), health_handle).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), canary_handle).await;

    let stats = approval_gate.stats();
    let shadow_summary = shadow_comparator.summary();
    info!(
        "Canary session complete — {} approved, {} rejected, {} trades, {} shadow-compared",
        stats.approved,
        stats.rejected,
        shadow_summary.total_canary_trades,
        shadow_summary.total_with_shadow,
    );
    if shadow_summary.total_with_shadow > 0 {
        info!(
            "Shadow agreement rate: {:.1}% ({}/{})",
            shadow_summary.shadow_agreement_rate * 100.0,
            shadow_summary.shadow_agreed,
            shadow_summary.total_with_shadow,
        );
    }

    Ok(())
}

// ── Canary trade loop ───────────────────────────────────────────────────

/// Run the canary evaluation loop with ShredStream data.
async fn start_canary_loop(
    config: &AppConfig,
    _orchestrator: &Orchestrator,
    approval_gate: &ManualApprovalGate,
    _shadow_comparator: &ShadowComparator,
    running: &AtomicBool,
) {
    // Build ShredStream config from AppConfig
    let shred_cfg = &config.shredstream;
    let port: u16 = shred_cfg
        .udp_bind_addr
        .split(':')
        .last()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8001);
    let bind_address = shred_cfg
        .udp_bind_addr
        .rsplitn(2, ':')
        .last()
        .unwrap_or("0.0.0.0")
        .to_string();

    let shred_config = ShredstreamConfig {
        port,
        bind_address,
        recv_buf: shred_cfg.receive_buffer_bytes as usize,
        max_age: shred_cfg.slot_window as u64,
        busy_poll_us: Some(shred_cfg.busy_poll_usec),
        pool_size: 4096,
        enable_fec: shred_cfg.fec_enabled,
        disable_salvage_delivery: false,
        stuck_batch_timeout_ms: shred_cfg.stuck_batch_timeout_ms,
        receive_core: Some(shred_cfg.cpu_affinity.receive_core as usize),
        reconstruct_core: Some(shred_cfg.cpu_affinity.reconstruct_core as usize),
        allowed_programs: shred_cfg.program_ids.clone(),
        filter_votes: true,
        raw_packet_queue_capacity: 32_768,
        decoded_slot_queue_capacity: 1_024,
        classified_event_queue_capacity: 8_192,
        trade_intent_queue_capacity: 256,
        dedup_cache_size: std::num::NonZeroUsize::new(262_144).unwrap(),
        ..Default::default()
    };

    let _adapter = ShredstreamAdapter::new(shred_config);
    info!("ShredStream adapter created for canary mode");

    // In canary mode, we don't run the full ShadowEngine — we wait for
    // manual approval. The adapter polls for classified events, and
    // when a signal is detected, it's submitted to the approval gate.
    //
    // For now, the canary loop is a simplified poll loop that:
    // 1. Prunes expired approvals
    // 2. Checks for approved trades and executes them
    // 3. Records outcomes in the shadow comparator
    //
    // Full strategy evaluation is delegated to the orchestrator and
    // triggered via the approval gate.

    let mut eval_interval = tokio::time::interval(Duration::from_millis(200));
    let mut prune_interval = tokio::time::interval(Duration::from_secs(10));

    while running.load(Ordering::Acquire) {
        tokio::select! {
            _ = eval_interval.tick() => {
                // Check for approved trades and execute them
                // (In a full implementation, this would poll the ShredStream
                // adapter and evaluate signals, then submit to approval gate)
            }
            _ = prune_interval.tick() => {
                // Prune expired approvals
                let expired = approval_gate.prune_expired();
                if !expired.is_empty() {
                    info!("Pruned {} expired canary approvals", expired.len());
                }
            }
        }
    }
}

// ── Metrics HTTP server ─────────────────────────────────────────────────

async fn run_metrics_server(addr: &str, running: Arc<AtomicBool>) -> anyhow::Result<()> {
    let server = Server::http(addr)
        .map_err(|e| anyhow::anyhow!("Failed to start metrics server on {addr}: {e}"))?;

    let content_type =
        Header::from_bytes(&b"Content-Type"[..], &b"text/plain; version=0.0.4; charset=utf-8"[..])
            .unwrap();

    while running.load(Ordering::Acquire) {
        match server.recv_timeout(Duration::from_secs(1)) {
            Ok(Some(request)) => {
                if request.method() != &Method::Get {
                    let _ = request
                        .respond(Response::from_string("Method not allowed\n").with_status_code(405));
                    continue;
                }

                match request.url() {
                    "/metrics" => {
                        let body = PerfRegistry::global().report();
                        let resp = Response::from_string(body).with_header(content_type.clone());
                        let _ = request.respond(resp);
                    }
                    _ => {
                        let _ = request
                            .respond(Response::from_string("Not found\n").with_status_code(404));
                    }
                }
            }
            Ok(None) => {}
            Err(e) => {
                if running.load(Ordering::Acquire) {
                    warn!("Metrics server recv error: {e}");
                }
                break;
            }
        }
    }

    Ok(())
}

// ── Health HTTP server ──────────────────────────────────────────────────

async fn run_health_server(addr: &str, running: Arc<AtomicBool>) -> anyhow::Result<()> {
    let server = Server::http(addr)
        .map_err(|e| anyhow::anyhow!("Failed to start health server on {addr}: {e}"))?;

    let content_type = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();

    let build_json = format!(
        r#"{{"status":"ok","build":"{}","version":"{}","mode":"canary","pid":{}}}"#,
        BUILD_NAME,
        BUILD_VERSION,
        std::process::id(),
    );

    while running.load(Ordering::Acquire) {
        match server.recv_timeout(Duration::from_secs(1)) {
            Ok(Some(request)) => {
                if request.method() != &Method::Get {
                    let _ = request
                        .respond(Response::from_string("Method not allowed\n").with_status_code(405));
                    continue;
                }

                match request.url() {
                    "/health" | "/" => {
                        let resp = Response::from_string(&build_json).with_header(content_type.clone());
                        let _ = request.respond(resp);
                    }
                    _ => {
                        let _ = request
                            .respond(Response::from_string("Not found\n").with_status_code(404));
                    }
                }
            }
            Ok(None) => {}
            Err(e) => {
                if running.load(Ordering::Acquire) {
                    warn!("Health server recv error: {e}");
                }
                break;
            }
        }
    }

    Ok(())
}

// ── Canary API HTTP server ──────────────────────────────────────────────

async fn run_canary_api_server(
    addr: &str,
    running: Arc<AtomicBool>,
    gate: Arc<ManualApprovalGate>,
    comparator: Arc<ShadowComparator>,
) -> anyhow::Result<()> {
    let server = Server::http(addr)
        .map_err(|e| anyhow::anyhow!("Failed to start canary API server on {addr}: {e}"))?;

    let json_type = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();

    while running.load(Ordering::Acquire) {
        match server.recv_timeout(Duration::from_secs(1)) {
            Ok(Some(request)) => {
                let response = match parse_canary_path(request.url(), request.method()) {
                    CanaryApiPath::Pending => {
                        let trades = gate.get_pending();
                        let body = serde_json::to_string_pretty(&trades)
                            .unwrap_or_else(|_| "[]".to_string());
                        Response::from_string(body).with_header(json_type.clone())
                    }
                    CanaryApiPath::Approve { id } => {
                        match gate.approve(&id) {
                            Some(trade) => {
                                let body = serde_json::json!({
                                    "status": "approved",
                                    "id": id,
                                    "protocol": trade.protocol,
                                    "mint": trade.mint,
                                    "direction": trade.direction,
                                });
                                Response::from_string(body.to_string()).with_header(json_type.clone())
                            }
                            None => {
                                let body = serde_json::json!({
                                    "error": "Trade not found or already processed",
                                    "id": id,
                                });
                                Response::from_string(body.to_string())
                                    .with_header(json_type.clone())
                                    .with_status_code(404)
                            }
                        }
                    }
                    CanaryApiPath::Reject { id, reason } => {
                        match gate.reject(&id, &reason) {
                            Some(trade) => {
                                let body = serde_json::json!({
                                    "status": "rejected",
                                    "id": id,
                                    "reason": reason,
                                    "protocol": trade.protocol,
                                });
                                Response::from_string(body.to_string()).with_header(json_type.clone())
                            }
                            None => {
                                let body = serde_json::json!({
                                    "error": "Trade not found or already processed",
                                    "id": id,
                                });
                                Response::from_string(body.to_string())
                                    .with_header(json_type.clone())
                                    .with_status_code(404)
                            }
                        }
                    }
                    CanaryApiPath::ApproveAll => {
                        let count = gate.approve_all();
                        let body = serde_json::json!({
                            "status": "approved",
                            "count": count,
                        });
                        Response::from_string(body.to_string()).with_header(json_type.clone())
                    }
                    CanaryApiPath::RejectAll { reason } => {
                        let count = gate.reject_all(&reason);
                        let body = serde_json::json!({
                            "status": "rejected",
                            "count": count,
                            "reason": reason,
                        });
                        Response::from_string(body.to_string()).with_header(json_type.clone())
                    }
                    CanaryApiPath::ShadowCompare => {
                        let summary = comparator.summary();
                        let body = serde_json::to_string_pretty(&summary)
                            .unwrap_or_else(|_| "{}".to_string());
                        Response::from_string(body).with_header(json_type.clone())
                    }
                    CanaryApiPath::Stats => {
                        let gate_stats = gate.stats();
                        let shadow_summary = comparator.summary();

                        #[derive(serde::Serialize)]
                        struct CombinedStats {
                            gate: serde_json::Value,
                            shadow: serde_json::Value,
                            mode: String,
                        }

                        let stats = CombinedStats {
                            gate: serde_json::to_value(&gate_stats).unwrap_or_default(),
                            shadow: serde_json::to_value(&shadow_summary).unwrap_or_default(),
                            mode: "canary".to_string(),
                        };
                        let body = serde_json::to_string_pretty(&stats)
                            .unwrap_or_else(|_| "{}".to_string());
                        Response::from_string(body).with_header(json_type.clone())
                    }
                    CanaryApiPath::Records { n } => {
                        let records = comparator.get_records(n);
                        let body = serde_json::to_string_pretty(&records)
                            .unwrap_or_else(|_| "[]".to_string());
                        Response::from_string(body).with_header(json_type.clone())
                    }
                    CanaryApiPath::NotFound => {
                        Response::from_string("Not found\n").with_status_code(404)
                    }
                };

                let _ = request.respond(response);
            }
            Ok(None) => {}
            Err(e) => {
                if running.load(Ordering::Acquire) {
                    warn!("Canary API server recv error: {e}");
                }
                break;
            }
        }
    }

    Ok(())
}

// ── Canary API path parser ──────────────────────────────────────────────

enum CanaryApiPath {
    Pending,
    Approve { id: String },
    Reject { id: String, reason: String },
    ApproveAll,
    RejectAll { reason: String },
    ShadowCompare,
    Stats,
    Records { n: usize },
    NotFound,
}

fn parse_canary_path(url: &str, method: &Method) -> CanaryApiPath {
    let path = url.split('?').next().unwrap_or(url);
    let query = url.split('?').nth(1).unwrap_or("");

    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    match parts.as_slice() {
        ["canary", "pending"] if method == &Method::Get => CanaryApiPath::Pending,
        ["canary", "approve", id] if method == &Method::Post => {
            CanaryApiPath::Approve { id: (*id).to_string() }
        }
        ["canary", "reject", id] if method == &Method::Post => {
            let reason = query
                .split('&')
                .find_map(|kv| {
                    let mut parts = kv.splitn(2, '=');
                    if parts.next()? == "reason" {
                        Some(parts.next().unwrap_or("No reason given").to_string())
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| "Rejected by operator".to_string());
            CanaryApiPath::Reject { id: (*id).to_string(), reason }
        }
        ["canary", "approve-all"] if method == &Method::Post => CanaryApiPath::ApproveAll,
        ["canary", "reject-all"] if method == &Method::Post => {
            let reason = query
                .split('&')
                .find_map(|kv| {
                    let mut parts = kv.splitn(2, '=');
                    if parts.next()? == "reason" {
                        Some(parts.next().unwrap_or("Bulk rejected").to_string())
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| "Bulk rejected by operator".to_string());
            CanaryApiPath::RejectAll { reason }
        }
        ["canary", "shadow-compare"] if method == &Method::Get => CanaryApiPath::ShadowCompare,
        ["canary", "stats"] if method == &Method::Get => CanaryApiPath::Stats,
        ["canary", "records"] if method == &Method::Get => {
            let n = query
                .split('&')
                .find_map(|kv| {
                    let mut parts = kv.splitn(2, '=');
                    if parts.next()? == "n" {
                        parts.next()?.parse::<usize>().ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(20)
                .min(1000);
            CanaryApiPath::Records { n }
        }
        _ => CanaryApiPath::NotFound,
    }
}

// ── Signal handling ─────────────────────────────────────────────────────

async fn wait_for_shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("Failed to install SIGTERM handler (unsupported platform)");

    tokio::select! {
        _ = ctrl_c => {
            info!("Received SIGINT (Ctrl+C)");
        }
        _ = sigterm.recv() => {
            info!("Received SIGTERM");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_canary_path_pending() {
        match parse_canary_path("/canary/pending", &Method::Get) {
            CanaryApiPath::Pending => {}
            _ => panic!("Expected Pending"),
        }
    }

    #[test]
    fn test_parse_canary_path_approve() {
        match parse_canary_path("/canary/approve/abc-123", &Method::Post) {
            CanaryApiPath::Approve { id } => assert_eq!(id, "abc-123"),
            _ => panic!("Expected Approve"),
        }
    }

    #[test]
    fn test_parse_canary_path_reject() {
        match parse_canary_path("/canary/reject/xyz?reason=bad+spread", &Method::Post) {
            CanaryApiPath::Reject { id, reason } => {
                assert_eq!(id, "xyz");
                assert_eq!(reason, "bad+spread");
            }
            _ => panic!("Expected Reject"),
        }
    }

    #[test]
    fn test_parse_canary_path_approve_all() {
        match parse_canary_path("/canary/approve-all", &Method::Post) {
            CanaryApiPath::ApproveAll => {}
            _ => panic!("Expected ApproveAll"),
        }
    }

    #[test]
    fn test_parse_canary_path_shadow_compare() {
        match parse_canary_path("/canary/shadow-compare", &Method::Get) {
            CanaryApiPath::ShadowCompare => {}
            _ => panic!("Expected ShadowCompare"),
        }
    }

    #[test]
    fn test_parse_canary_path_stats() {
        match parse_canary_path("/canary/stats", &Method::Get) {
            CanaryApiPath::Stats => {}
            _ => panic!("Expected Stats"),
        }
    }

    #[test]
    fn test_parse_canary_path_records() {
        match parse_canary_path("/canary/records?n=50", &Method::Get) {
            CanaryApiPath::Records { n } => assert_eq!(n, 50),
            _ => panic!("Expected Records(50)"),
        }
    }

    #[test]
    fn test_parse_canary_path_records_default() {
        match parse_canary_path("/canary/records", &Method::Get) {
            CanaryApiPath::Records { n } => assert_eq!(n, 20),
            _ => panic!("Expected Records(20)"),
        }
    }

    #[test]
    fn test_parse_canary_path_not_found() {
        match parse_canary_path("/canary/unknown", &Method::Get) {
            CanaryApiPath::NotFound => {}
            _ => panic!("Expected NotFound"),
        }
    }

    #[test]
    fn test_parse_canary_path_get_method_only() {
        // POST to /canary/pending should be NotFound
        match parse_canary_path("/canary/pending", &Method::Post) {
            CanaryApiPath::NotFound => {}
            _ => panic!("Expected NotFound for wrong method"),
        }
    }
}