use sol_trade_sdk::{
    common::gas_fee_strategy::GasFeeStrategy,
    swqos::{SwqosType, TradeType},
};

#[tokio::main]
async fn main() {
    println!("🚀 Gas Fee Strategy Demo");
    println!("========================");

    // Set global strategy
    println!("1. Set global strategy");
    GasFeeStrategy::set_global_fee_strategy(150000, 500000, 0.001, 0.001);

    // Print all strategies
    println!("\n2. Print all strategies");
    GasFeeStrategy::print_all_strategies();

    // Clear all strategies
    println!("\n3. Clear all strategies");
    GasFeeStrategy::clear();

    // Add normal fee strategy for SwqosType::Default
    println!("\n4. Add normal fee strategy for SwqosType::Default");
    GasFeeStrategy::set_normal_fee_strategy(
        SwqosType::Default,
        150000, // cu_limit
        500000, // cu_price
        0.0,    // buy_tip
        0.0,    // sell_tip
    );

    // Add high-low fee strategy for SwqosType::Jito on Buy
    println!("\n5. Add high-low fee strategy for SwqosType::Jito on Buy");
    GasFeeStrategy::set_high_low_fee_strategy(
        SwqosType::Jito,
        TradeType::Buy,
        150000,         // cu_limit
        100,            // low cu_price
        10 * 1_000_000, // high cu_price
        0.001,          // low tip
        0.1,            // high tip
    );

    // Print all strategies
    println!("\n6. Print all current strategies");
    GasFeeStrategy::print_all_strategies();

    // Add normal fee strategy for SwqosType::Jito on Buy (will override previous high-low strategy)
    println!("\n7. Add normal fee strategy for SwqosType::Jito (will override previous high-low strategy)");
    GasFeeStrategy::set_normal_fee_strategy(
        SwqosType::Jito,
        150000, // cu_limit
        500000, // cu_price
        0.0001, // buy_tip
        0.0001, // sell_tip
    );

    // Print all strategies
    println!("\n8. Print all current strategies");
    GasFeeStrategy::print_all_strategies();

    // Remove strategy for SwqosType::Jito on Buy
    println!("\n9. Remove strategy for SwqosType::Jito on Buy");
    GasFeeStrategy::del(SwqosType::Jito, TradeType::Buy);

    // Print all strategies
    println!("\n10. Print all current strategies");
    GasFeeStrategy::print_all_strategies();

    println!("\n✅ Gas Fee Strategy Demo completed!");
}
