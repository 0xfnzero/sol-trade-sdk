//! Shadow Mode binary — live feed, no submission, outcome comparison.
//!
//! Phase 11 of the productionization roadmap: connect to live ShredStream data,
//! run the full evaluation pipeline, generate hypothetical trade decisions,
//! and track accuracy against actual market movement — all without submitting
//! any transactions.
//!
//! No wallet keypair is required. The binary shares the existing Solana SDK
//! and ShredStream infrastructure but operates in observation-only mode.
//!
//! # Usage
//!
//! ```text
//! solshadow --config /etc/solbot/config.toml
//!           [--metrics  127.0.0.1:9090]
//!           [--health   127.0.0.1:9091]
//!           [--shadow   127.0.0.1:9092]
//! ```
//!
//! # HTTP Endpoints
//!
//! | Path | Description |
//! |------|-------------|
//! | `GET /health` | Build info + shadow status |
//! | `GET /metrics` | Prometheus-format counters from PerfRegistry |
//! | `GET /shadow/decisions?n=20` | Last N shadow decisions as JSON |
//! | `GET /shadow/accuracy` | Aggregate accuracy stats across 2s/5s/10s/30s horizons |
//! | `GET /shadow/stats` | Summary counters |

use std::env;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use tiny_http::{Header, Method, Response, Server};
use tracing::{error, info, warn};

use sol_trade_sdk::common::config::AppConfig;
use sol_trade_sdk::perf::shredstream::config::ShredstreamConfig;
use sol_trade_sdk::perf::shredstream::ShredstreamAdapter;
use sol_trade_sdk::perf::PerfRegistry;
use sol_trade_sdk::trading::shadow::{AccuracyChecker, DecisionBuffer, ShadowEngine};

// ── Build info ──────────────────────────────────────────────────────────

const BUILD_VERSION: &str = env!("CARGO_PKG_VERSION");
const BUILD_NAME: &str = "solshadow";

// ── CLI argument parsing ────────────────────────────────────────────────

struct Cli {
    config: PathBuf,
    metrics_addr: String,
    health_addr: String,
    shadow_addr: String,
}

fn parse_args() -> Cli {
    let args: Vec<String> = env::args().collect();
    let mut cli = Cli {
        config: PathBuf::from("/etc/solbot/config.toml"),
        metrics_addr: "127.0.0.1:9090".to_string(),
        health_addr: "127.0.0.1:9091".to_string(),
        shadow_addr: "127.0.0.1:9092".to_string(),
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
            "--shadow" | "-s" => {
                i += 1;
                if i < args.len() {
                    cli.shadow_addr = args[i].clone();
                }
            }
            "--help" | "-h" => {
                eprintln!("solshadow — Shadow Mode DEX Observer");
                eprintln!();
                eprintln!("Connects to live ShredStream data, evaluates trading strategies,");
                eprintln!("records hypothetical decisions, and tracks accuracy — without");
                eprintln!("submitting any transactions or requiring a wallet keypair.");
                eprintln!();
                eprintln!("USAGE:");
                eprintln!("  solshadow [OPTIONS]");
                eprintln!();
                eprintln!("OPTIONS:");
                eprintln!("  -c, --config PATH      Config file path [default: /etc/solbot/config.toml]");
                eprintln!("  -m, --metrics ADDR     Metrics HTTP listen [default: 127.0.0.1:9090]");
                eprintln!("  -H, --health ADDR      Health HTTP listen  [default: 127.0.0.1:9091]");
                eprintln!("  -s, --shadow ADDR      Shadow API HTTP       [default: 127.0.0.1:9092]");
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

    info!("{BUILD_NAME} v{BUILD_VERSION} starting — shadow mode, no submission");

    // 3. Load configuration (minimal — only need ShredStream + network config)
    let config_str = std::fs::read_to_string(&cli.config)
        .with_context(|| format!("Failed to read config: {}", cli.config.display()))?;

    let app_config: AppConfig = toml::from_str(&config_str)
        .with_context(|| format!("Failed to parse config: {}", cli.config.display()))?;

    app_config.validate().context("Configuration validation failed")?;

    info!("Configuration loaded and validated from {}", cli.config.display());

    // 4. Create ShredStream adapter from config (map config fields to ShredstreamConfig)
    let shred_cfg = &app_config.shredstream;
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

    let s_port = shred_config.port;
    let s_buf = shred_config.recv_buf;
    let adapter = ShredstreamAdapter::new(shred_config);
    info!("ShredStream adapter created (port: {}, buf: {})", s_port, s_buf);

    // 5. Create ShadowEngine (no keypair needed — purely observational)
    let mut engine = ShadowEngine::new(adapter);

    let decisions: Arc<DecisionBuffer> = Arc::clone(engine.decisions());
    let accuracy: Arc<AccuracyChecker> = Arc::clone(engine.accuracy());
    let running = Arc::new(AtomicBool::new(true));

    // 6. Start HTTP servers (metrics, health, shadow API)
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

    let shadow_handle = {
        let running = running.clone();
        let addr = cli.shadow_addr.clone();
        let decisions = Arc::clone(&decisions);
        let accuracy = Arc::clone(&accuracy);
        tokio::spawn(async move {
            if let Err(e) = run_shadow_api_server(&addr, running, decisions, accuracy).await {
                error!("Shadow API server failed: {e}");
            }
        })
    };

    info!("Metrics HTTP:      http://{}", cli.metrics_addr);
    info!("Health HTTP:       http://{}", cli.health_addr);
    info!("Shadow API HTTP:   http://{}", cli.shadow_addr);
    info!("No wallet keypair loaded — observation mode only");

    // 7. Run the shadow engine (blocks until ShredStream disconnects)
    engine.run().await;

    // 8. Graceful shutdown
    info!("Shutdown initiated — stopping HTTP servers");
    running.store(false, Ordering::Release);

    let _ = tokio::time::timeout(Duration::from_secs(5), metrics_handle).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), health_handle).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), shadow_handle).await;

    info!(
        "Shadow session complete — {} decisions recorded, {} evaluated",
        decisions.total_decisions(),
        accuracy.snapshot().total_evaluated,
    );

    Ok(())
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
        r#"{{"status":"ok","build":"{}","version":"{}","mode":"shadow","pid":{}}}"#,
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

// ── Shadow API HTTP server ──────────────────────────────────────────────

async fn run_shadow_api_server(
    addr: &str,
    running: Arc<AtomicBool>,
    decisions: Arc<DecisionBuffer>,
    accuracy: Arc<AccuracyChecker>,
) -> anyhow::Result<()> {
    let server = Server::http(addr)
        .map_err(|e| anyhow::anyhow!("Failed to start shadow API server on {addr}: {e}"))?;

    let json_type = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();

    while running.load(Ordering::Acquire) {
        match server.recv_timeout(Duration::from_secs(1)) {
            Ok(Some(request)) => {
                if request.method() != &Method::Get {
                    let _ = request
                        .respond(Response::from_string("Method not allowed\n").with_status_code(405));
                    continue;
                }

                let response = match parse_shadow_path(request.url()) {
                    ShadowApiPath::Decisions { n } => {
                        let decs = decisions.recent(n);
                        let body = serde_json::to_string_pretty(&decs)
                            .unwrap_or_else(|_| "[]".to_string());
                        Response::from_string(body).with_header(json_type.clone())
                    }
                    ShadowApiPath::Accuracy => {
                        let snap = accuracy.snapshot();
                        let body = serde_json::to_string_pretty(&snap)
                            .unwrap_or_else(|_| "{}".to_string());
                        Response::from_string(body).with_header(json_type.clone())
                    }
                    ShadowApiPath::Stats => {
                        let snap = accuracy.snapshot();
                        #[derive(serde::Serialize)]
                        struct StatsResponse {
                            total_decisions: u64,
                            total_directional: u64,
                            total_evaluated: u64,
                            total_expired: u64,
                            pending_checks: usize,
                            buffer_occupancy: usize,
                            mode: String,
                        }
                        let stats = StatsResponse {
                            total_decisions: decisions.total_decisions(),
                            total_directional: snap.total_directional,
                            total_evaluated: snap.total_evaluated,
                            total_expired: snap.total_expired,
                            pending_checks: accuracy.pending_count(),
                            buffer_occupancy: decisions.len(),
                            mode: "shadow".to_string(),
                        };
                        let body = serde_json::to_string_pretty(&stats)
                            .unwrap_or_else(|_| "{}".to_string());
                        Response::from_string(body).with_header(json_type.clone())
                    }
                    ShadowApiPath::NotFound => {
                        Response::from_string("Not found\n").with_status_code(404)
                    }
                };

                let _ = request.respond(response);
            }
            Ok(None) => {}
            Err(e) => {
                if running.load(Ordering::Acquire) {
                    warn!("Shadow API server recv error: {e}");
                }
                break;
            }
        }
    }

    Ok(())
}

// ── Shadow API path parser ──────────────────────────────────────────────

enum ShadowApiPath {
    Decisions { n: usize },
    Accuracy,
    Stats,
    NotFound,
}

fn parse_shadow_path(url: &str) -> ShadowApiPath {
    // Remove query string
    let path = url.split('?').next().unwrap_or(url);
    let query = if let Some(q) = url.split('?').nth(1) { q } else { "" };

    match path {
        "/shadow/decisions" | "/decisions" => {
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
            ShadowApiPath::Decisions { n }
        }
        "/shadow/accuracy" | "/accuracy" => ShadowApiPath::Accuracy,
        "/shadow/stats" | "/stats" => ShadowApiPath::Stats,
        _ => {
            // Also support /shadow prefix on its own
            if path == "/shadow" || path == "/" || path == "/health" {
                // Redirect to accuracy by default
                ShadowApiPath::Stats
            } else {
                ShadowApiPath::NotFound
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_shadow_path_decisions() {
        match parse_shadow_path("/shadow/decisions?n=50") {
            ShadowApiPath::Decisions { n } => assert_eq!(n, 50),
            _ => panic!("Expected Decisions"),
        }
    }

    #[test]
    fn test_parse_shadow_path_decisions_default_n() {
        match parse_shadow_path("/shadow/decisions") {
            ShadowApiPath::Decisions { n } => assert_eq!(n, 20),
            _ => panic!("Expected Decisions(20)"),
        }
    }

    #[test]
    fn test_parse_shadow_path_accuracy() {
        match parse_shadow_path("/shadow/accuracy") {
            ShadowApiPath::Accuracy => {}
            _ => panic!("Expected Accuracy"),
        }
    }

    #[test]
    fn test_parse_shadow_path_stats() {
        match parse_shadow_path("/shadow/stats") {
            ShadowApiPath::Stats => {}
            _ => panic!("Expected Stats"),
        }
    }

    #[test]
    fn test_parse_shadow_path_not_found() {
        match parse_shadow_path("/some/other/path") {
            ShadowApiPath::NotFound => {}
            _ => panic!("Expected NotFound"),
        }
    }
}