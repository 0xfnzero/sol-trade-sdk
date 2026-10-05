//! Compare IDL-built deposit/withdraw with and without creator-fee collection.
//! Only unsigned simulateTransaction calls; no transaction is broadcast.
#[path = "support/cpmm_lp_collection_simulation.rs"]
mod simulation;
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let report = simulation::run_from_env().await?;
    for row in report["comparisons"].as_array().unwrap() {
        println!(
            "permissionless={} withdraw={} LP={} token0_delta={} token1_delta={} lp_delta={} units={}/{} validated=true",
            row["permissionless"],
            row["withdraw"],
            row["lp_amount"],
            row["token0_delta"],
            row["token1_delta"],
            row["lp_delta"],
            row["baseline_units"],
            row["collected_units"]
        );
    }
    Ok(())
}
