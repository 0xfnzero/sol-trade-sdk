//! Offline stock-route construction; --live loads current accounts without simulation.
#[path = "support/pumpfun_example.rs"]
mod pumpfun_example;
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    pumpfun_example::run(std::env::args().skip(1).collect(), false).await
}
