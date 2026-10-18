//! Shared RPC-free demo transaction settings; no wallet funds or network.
use sol_trade_sdk::{
    common::{GasFeeStrategy, TradeTransactionVersion},
    swqos::TradeType,
    trading::core::params::{DexParamEnum, SwapParams},
};
use solana_sdk::{hash::Hash, pubkey::Pubkey, signature::Keypair};
use std::sync::Arc;
pub fn offline_demo_params(version: TradeTransactionVersion, close_output: bool) -> SwapParams {
    SwapParams {
        rpc: None,
        payer: Arc::new(Keypair::new()),
        trade_type: TradeType::Buy, // PreparedPumpSwapTrade replaces trade fields.
        input_mint: Pubkey::default(),
        input_token_program: None,
        output_mint: Pubkey::default(),
        output_token_program: None,
        input_amount: None,
        slippage_basis_points: None,
        address_lookup_table_accounts: vec![],
        recent_blockhash: Some(Hash::new_unique()),
        wait_tx_confirmed: false,
        protocol_params: DexParamEnum::PumpSwapQuoteRoute(Default::default()),
        open_seed_optimize: true, // PumpSwap routes still use idempotent ATAs.
        swqos_clients: Arc::new(vec![]),
        middleware_manager: None,
        durable_nonce: None,
        with_tip: false,
        create_input_mint_ata: true,
        close_input_mint_ata: false,
        create_output_mint_ata: true,
        close_output_mint_ata: close_output,
        fixed_output_amount: None,
        gas_fee_strategy: GasFeeStrategy::new(),
        simulate: false,
        log_enabled: false,
        wait_for_all_submits: false,
        use_dedicated_sender_threads: false,
        sender_thread_cores: None,
        max_sender_concurrency: 0,
        effective_core_ids: Arc::new(vec![]),
        check_min_tip: false,
        transaction_version: version,
        grpc_recv_us: None,
        use_exact_sol_amount: None,
    }
}
