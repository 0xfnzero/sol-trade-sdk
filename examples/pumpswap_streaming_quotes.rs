//! Cached quotes from a selected transaction stream. Offline replay by default.
//! --help lists sources and live configuration; see docs/STREAMING_QUOTES.md.
#[path = "support/streaming_quotes.rs"]
mod demo;
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    demo::run().await
}
