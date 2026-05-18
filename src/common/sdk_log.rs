//! sol-trade-sdk global log switch
//!
//! Controlled by `TradeConfig::log_enabled`, set in `TradingClient::new`.
//! All SDK logs (timing, SWQOS submit/confirm, WSOL, blacklist, etc.) should check this before output.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Format duration for log: "97.9396 ms" or "13.936 µs", 4 decimal places, space before unit.
fn format_elapsed(d: Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 0.001 {
        format!("{:.4} µs", secs * 1_000_000.0)
    } else {
        format!("{:.4} ms", secs * 1000.0)
    }
}

/// Extract a short error message for SWQOS submission failed log.
/// Tries JSON "message"/"data" and quoted string; on any parse failure returns original (no panic).
fn extract_swqos_error_message(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() {
        return String::new();
    }
    // Plain double-quoted string (no inner JSON): unquote
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        let inner = &s[1..s.len() - 1];
        if !inner.contains('{') {
            return inner.replace("\\\"", "\"");
        }
    }
    // Try parse as JSON only when input looks like JSON (avoid parsing long non-JSON strings)
    if s.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(s) {
            let obj = v.get("error").and_then(|e| e.as_object()).or_else(|| v.as_object());
            if let Some(o) = obj {
                if let Some(m) = o.get("message").and_then(|x| x.as_str()) {
                    return m.to_string();
                }
                if let Some(d) = o.get("data").and_then(|x| x.as_str()) {
                    return d.to_string();
                }
            }
        }
    }
    s.to_string()
}

static SDK_LOG_ENABLED: AtomicBool = AtomicBool::new(true);

/// Width of [provider] label so SWQOS submit/confirm lines align (longest: Speedlanding).
pub const SWQOS_LABEL_WIDTH: usize = 12;

/// Whether SDK logging is enabled (set from TradeConfig.log_enabled in TradingClient::new).
#[inline(always)]
pub fn sdk_log_enabled() -> bool {
    SDK_LOG_ENABLED.load(Ordering::Relaxed)
}

/// Set the SDK global log switch (only called from TradingClient::new).
pub fn set_sdk_log_enabled(enabled: bool) {
    SDK_LOG_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Aligned log: ` [Soyas        ] Buy submitted: 13.936 µs`. Call only when sdk_log_enabled().
#[inline]
pub fn log_swqos_submitted(provider: &str, trade_type: impl fmt::Display, elapsed: Duration) {
    println!(
        " [{:width$}] {} submitted: {}",
        provider,
        trade_type,
        format_elapsed(elapsed),
        width = SWQOS_LABEL_WIDTH
    );
}

/// Prints one SDK timing block (build_instructions, before_submit, per-channel submit_done).
/// When confirm_us is Some, prints confirmed + total; when None, prints "confirmed: -, total: submit_ms".
/// Call only when sdk_log_enabled().
pub fn print_sdk_timing_block(
    dir: &str,
    start_us: Option<i64>,
    build_end_us: Option<i64>,
    before_submit_us: Option<i64>,
    submit_timings: &[crate::common::SwqosSubmitTiming],
    confirm_us: Option<i64>,
) {
    println!();
    let start_us = match start_us {
        Some(u) => u,
        None => return,
    };
    if let Some(end_us) = build_end_us {
        println!(
            " [SDK][{:width$}] {} build_instructions: {:.4} ms",
            "-",
            dir,
            (end_us - start_us) as f64 / 1000.0,
            width = SWQOS_LABEL_WIDTH
        );
    }
    if let Some(end_us) = before_submit_us {
        println!(
            " [SDK][{:width$}] {} before_submit: {:.4} ms",
            "-",
            dir,
            (end_us - start_us) as f64 / 1000.0,
            width = SWQOS_LABEL_WIDTH
        );
    }
    if let Some(confirm_done_us) = confirm_us {
        let total_ms = (confirm_done_us - start_us) as f64 / 1000.0;
        for timing in submit_timings {
            let submit_ms = (timing.submit_done_us - start_us).max(0) as f64 / 1000.0;
            let confirmed_ms = (confirm_done_us - timing.submit_done_us).max(0) as f64 / 1000.0;
            println!(
                " [SDK][{:width$}] {} {} submit_done: {:.4} ms, confirmed: {:.4} ms, total: {:.4} ms",
                timing.swqos_type.as_str(),
                dir,
                timing.strategy_type.as_str(),
                submit_ms,
                confirmed_ms,
                total_ms,
                width = SWQOS_LABEL_WIDTH
            );
        }
    } else {
        for timing in submit_timings {
            let submit_ms = (timing.submit_done_us - start_us).max(0) as f64 / 1000.0;
            println!(
                " [SDK][{:width$}] {} {} submit_done: {:.4} ms, confirmed: -, total: {:.4} ms",
                timing.swqos_type.as_str(),
                dir,
                timing.strategy_type.as_str(),
                submit_ms,
                submit_ms,
                width = SWQOS_LABEL_WIDTH
            );
        }
    }
}

/// Aligned log: ` [Stellium     ] Buy submission failed after 97.9396 ms: ...`. Call only when sdk_log_enabled().
/// Error is normalized: JSON "message"/"data" or quoted string is shown; raw JSON is not.
#[inline]
pub fn log_swqos_submission_failed(
    provider: &str,
    trade_type: impl fmt::Display,
    elapsed: Duration,
    err: impl fmt::Display,
) {
    let msg = extract_swqos_error_message(&format!("{}", err));
    eprintln!(
        " [{:width$}] {} submission failed after {}, error: {}",
        provider,
        trade_type,
        format_elapsed(elapsed),
        msg,
        width = SWQOS_LABEL_WIDTH
    );
}
