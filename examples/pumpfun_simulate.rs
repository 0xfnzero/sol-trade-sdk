//! V1/V2/V3 direct trades and V2/V3 stock routes against current RPC state.
//! Simulation only: no signing or submission. See --help.
#[path = "support/pumpfun_example.rs"]
mod pumpfun_example;
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    pumpfun_example::run(std::env::args().skip(1).collect(), true).await
}
