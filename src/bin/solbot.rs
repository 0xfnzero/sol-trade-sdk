//! Production binary entrypoint for sol-trade-sdk.
//!
//! # Usage
//!
//! ```text
//! solbot --config /etc/solbot/config.toml [--keypair /path/to/keypair.json]
//!        [--metrics 127.0.0.1:9090] [--health 127.0.0.1:9091]
//! ```
//!
//! # Signals
//!
//! - `SIGTERM` / `SIGINT` (Ctrl+C) → graceful shutdown:
//!   kill switch → wait for in-flight trades → stop HTTP servers → exit
//!
//! # HTTP Endpoints
//!
//! - `GET /health` → `200 OK` with build info
//! - `GET /metrics` → Prometheus-format metrics from PerfRegistry

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
use sol_trade_sdk::trading::core::orchestrator::Orchestrator;
use sol_trade_sdk::trading::factory::DexType;
use sol_trade_sdk::trading::jito::{bundle_executor, TipConfig};
use sol_trade_sdk::swqos::SwqosClient;

// ── Build info (injected at compile time) ───────────────────────────────────

const BUILD_VERSION: &str = env!("CARGO_PKG_VERSION");
const BUILD_NAME: &str = env!("CARGO_PKG_NAME");

// ── CLI argument parsing (no extra deps) ────────────────────────────────────

struct Cli {
    config: PathBuf,
    keypair: Option<PathBuf>,
    metrics_addr: String,
    health_addr: String,
}

fn parse_args() -> Cli {
    let args: Vec<String> = env::args().collect();
    let mut cli = Cli {
        config: PathBuf::from("/etc/solbot/config.toml"),
        keypair: None,
        metrics_addr: "127.0.0.1:9090".to_string(),
        health_addr: "127.0.0.1:9091".to_string(),
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
            "--help" | "-h" => {
                eprintln!("Usage: solbot [OPTIONS]");
                eprintln!();
                eprintln!("Options:");
                eprintln!("  -c, --config PATH    Config file path [default: /etc/solbot/config.toml]");
                eprintln!("  -k, --keypair PATH   Keypair file (overrides config path)");
                eprintln!("  -m, --metrics ADDR   Metrics HTTP listen address [default: 127.0.0.1:9090]");
                eprintln!("  -H, --health ADDR    Health HTTP listen address [default: 127.0.0.1:9091]");
                eprintln!("  -h, --help           Show this help");
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

// ── Main ────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ------------------------------------------------------------------
    // 1. Parse CLI args
    // ------------------------------------------------------------------
    let cli = parse_args();

    // ------------------------------------------------------------------
    // 2. Setup tracing / logging
    // ------------------------------------------------------------------
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,sol_trade_sdk=info")),
        )
        .init();

    info!("{} v{} starting", BUILD_NAME, BUILD_VERSION);

    // ------------------------------------------------------------------
    // 3. Load configuration
    // ------------------------------------------------------------------
    let config_path = &cli.config;
    let config_str = std::fs::read_to_string(config_path)
        .with_context(|| format!("Failed to read config: {}", config_path.display()))?;

    let mut app_config: AppConfig = toml::from_str(&config_str)
        .with_context(|| format!("Failed to parse config: {}", config_path.display()))?;

    // Override keypair path from CLI if provided
    if let Some(ref kp) = cli.keypair {
        let path = kp.to_string_lossy().to_string();
        app_config.wallet.execution_keypair_path = path.clone();
        app_config.wallet.payer_keypair_path = path;
    }

    // Validate config
    app_config.validate().context("Configuration validation failed")?;

    info!("Configuration loaded and validated from {}", config_path.display());

    // ------------------------------------------------------------------
    // 4. Load keypair
    // ------------------------------------------------------------------
    let keypair_path = &app_config.wallet.execution_keypair_path;
    let keypair = read_keypair_file(keypair_path)
        .map_err(|e| anyhow::anyhow!("Failed to read keypair from {}: {}", keypair_path, e))?;

    let payer = Arc::new(keypair);
    info!("Wallet loaded: {}", payer.as_ref().pubkey());

    // ------------------------------------------------------------------
    // 5. Create Orchestrator (wrapped in Arc for shared access)
    // ------------------------------------------------------------------
    let orchestrator =
        Orchestrator::from_config(app_config.clone(), payer).context("Failed to create orchestrator")?;
    let orchestrator = Arc::new(orchestrator);

    // ------------------------------------------------------------------
    // 6. Start background services (blockhash + fee refresh)
    // ------------------------------------------------------------------
    orchestrator.start();
    info!("Orchestrator started — background services running");

    // ------------------------------------------------------------------
    // 7. Start HTTP servers (metrics + health on dedicated threads)
    // ------------------------------------------------------------------
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

    info!("Metrics HTTP: http://{}", cli.metrics_addr);
    info!("Health HTTP:  http://{}", cli.health_addr);

    // ------------------------------------------------------------------
    // 8. Start ShredStream event-ingest pipeline (if configured)
    // ------------------------------------------------------------------
    let shred_cfg = &app_config.shredstream;
    let _event_ingest_handle = if shred_cfg.enabled {
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

        let mut adapter = ShredstreamAdapter::new(shred_config);
        if let Err(e) = adapter.start() {
            warn!("ShredStream adapter failed to start: {e} — running without event feed");
            None
        } else {
            info!("ShredStream pipeline started — feeding classified events into strategy engine");
            let orch_ingest = Arc::clone(&orchestrator);
            let rx = adapter.classified_event_rx().clone();

            // Spawn the event-ingest task
            let handle = tokio::spawn(async move {
                loop {
                    match rx.recv_timeout(std::time::Duration::from_millis(100)) {
                        Ok(event) => {
                            orch_ingest.process_event(&event);
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                            // Normal — no events this cycle
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                            info!("ShredStream channel disconnected — event ingest ending");
                            break;
                        }
                    }
                }
                info!("Event-ingest loop terminated");
            });

            Some(handle)
        }
    } else {
        info!("ShredStream is disabled — running without event feed (strategy engine will have no market state)");
        None
    };

    // ------------------------------------------------------------------
    // 9a. Start Jito bundle trade execution loop (if configured)
    // ------------------------------------------------------------------
    let _trade_execution_handle = {
        let jito_cfg = &app_config.submission.jito;
        if jito_cfg.endpoint != "disabled" {
            // Build SWQOS clients from config
            let swqos_configs = app_config.submission.to_swqos_configs();
            let commitment = app_config.rpc.commitment_config();
            let mut clients: Vec<Arc<SwqosClient>> = Vec::new();
            for sc in &swqos_configs {
                if !sc.is_blacklisted() {
                    if let Ok(c) = sol_trade_sdk::swqos::SwqosConfig::get_swqos_client(
                        app_config.rpc.primary.clone(),
                        commitment,
                        sc.clone(),
                        false,
                    )
                    .await
                    {
                        clients.push(c);
                    }
                }
            }

            if !clients.is_empty() {
                let swqos_clients = Arc::new(clients);
                let tip_config = TipConfig::from_jito_config(jito_cfg);
                // Use a placeholder mint — real config should specify target mints
                // TODO: add target_mints to StrategyConfig in config.rs
                let target_mint = solana_sdk::pubkey::Pubkey::new_from_array([0u8; 32]);

                let rpc = orchestrator.rpc.clone();
                let orch = Arc::clone(&orchestrator);
                let running_clone = running.clone();
                let cfg_clone = app_config.clone();

                let handle = bundle_executor::spawn_trade_execution_loop(
                    orch,
                    DexType::Bonk, // Placeholder — configurable per target mint
                    target_mint,
                    tip_config,
                    swqos_clients,
                    rpc,
                    cfg_clone,
                    running_clone,
                );
                info!("Jito trade execution loop started");
                Some(handle)
            } else {
                warn!("No SWQOS clients available — trade execution loop not started");
                None
            }
        } else {
            info!("Jito is disabled (endpoint='disabled') — trade execution loop not started");
            None
        }
    };

    // ------------------------------------------------------------------
    // 10. Wait for shutdown signal (SIGTERM or Ctrl+C)
    // ------------------------------------------------------------------
    wait_for_shutdown_signal().await;
    info!("Shutdown signal received — initiating graceful shutdown");

    // Step 1: Activate kill switch — reject all new trade plans
    orchestrator.kill();
    info!("Kill switch activated — no new trades accepted");

    // Step 2: Signal HTTP servers to stop
    running.store(false, Ordering::Release);

    // Step 3: Brief grace period for in-flight trades
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Step 4: Wait for HTTP servers to finish (best-effort with timeout)
    let _ = tokio::time::timeout(Duration::from_secs(5), metrics_handle).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), health_handle).await;

    info!("Graceful shutdown complete. Goodbye.");
    Ok(())
}

// ── Metrics HTTP server ─────────────────────────────────────────────────────

/// Serves Prometheus-format metrics from PerfRegistry at GET /metrics.
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
                    let _ = request.respond(Response::from_string("Method not allowed\n").with_status_code(405));
                    continue;
                }

                match request.url() {
                    "/metrics" => {
                        let body = PerfRegistry::global().report();
                        let resp = Response::from_string(body).with_header(content_type.clone());
                        let _ = request.respond(resp);
                    }
                    _ => {
                        let _ = request.respond(Response::from_string("Not found\n").with_status_code(404));
                    }
                }
            }
            Ok(None) => { /* timeout, loop back to check running */ }
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

// ── Health HTTP server ──────────────────────────────────────────────────────

/// Serves a JSON health check at GET /health or GET /.
async fn run_health_server(addr: &str, running: Arc<AtomicBool>) -> anyhow::Result<()> {
    let server = Server::http(addr)
        .map_err(|e| anyhow::anyhow!("Failed to start health server on {addr}: {e}"))?;

    let content_type = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();

    let build_json = format!(
        r#"{{"status":"ok","build":"{}","version":"{}","pid":{}}}"#,
        BUILD_NAME,
        BUILD_VERSION,
        std::process::id(),
    );

    while running.load(Ordering::Acquire) {
        match server.recv_timeout(Duration::from_secs(1)) {
            Ok(Some(request)) => {
                if request.method() != &Method::Get {
                    let _ = request.respond(Response::from_string("Method not allowed\n").with_status_code(405));
                    continue;
                }

                match request.url() {
                    "/health" | "/" => {
                        let resp = Response::from_string(&build_json).with_header(content_type.clone());
                        let _ = request.respond(resp);
                    }
                    _ => {
                        let _ = request.respond(Response::from_string("Not found\n").with_status_code(404));
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

// ── Signal handling ─────────────────────────────────────────────────────────

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