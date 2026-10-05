//! Compare exact-input and exact-output swaps with/without creator-fee collection.
//! Only unsigned mainnet simulateTransaction; nothing is broadcast.
#[path = "support/cpmm_swap_collection_simulation.rs"]
mod simulation;
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(&simulation::run_from_env().await?)?);
    Ok(())
}
