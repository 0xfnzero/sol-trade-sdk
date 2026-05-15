use crate::common::SolanaRpcClient;
use crate::trading::common::get_multi_token_balances;
use solana_sdk::pubkey::Pubkey;

/// RaydiumCpmm protocol specific parameters
/// Configuration parameters specific to Raydium CPMM trading protocol
#[derive(Clone)]
pub struct RaydiumAmmV4Params {
    /// AMM pool address
    pub amm: Pubkey,
    /// Base token (coin) mint address
    pub coin_mint: Pubkey,
    /// Quote token (pc) mint address  
    pub pc_mint: Pubkey,
    /// Pool's coin token account address
    pub token_coin: Pubkey,
    /// Pool's pc token account address
    pub token_pc: Pubkey,
    /// AMM open orders account
    pub amm_open_orders: Pubkey,
    /// AMM target orders account
    pub amm_target_orders: Pubkey,
    /// Serum/OpenBook program used by the AMM market
    pub serum_program: Pubkey,
    /// Serum/OpenBook market account
    pub serum_market: Pubkey,
    /// Serum/OpenBook bids account
    pub serum_bids: Pubkey,
    /// Serum/OpenBook asks account
    pub serum_asks: Pubkey,
    /// Serum/OpenBook event queue account
    pub serum_event_queue: Pubkey,
    /// Serum/OpenBook coin vault account
    pub serum_coin_vault_account: Pubkey,
    /// Serum/OpenBook pc vault account
    pub serum_pc_vault_account: Pubkey,
    /// Serum/OpenBook vault signer PDA
    pub serum_vault_signer: Pubkey,
    /// Current coin reserve amount in the pool
    pub coin_reserve: u64,
    /// Current pc reserve amount in the pool
    pub pc_reserve: u64,
}

impl RaydiumAmmV4Params {
    pub fn new(
        amm: Pubkey,
        coin_mint: Pubkey,
        pc_mint: Pubkey,
        token_coin: Pubkey,
        token_pc: Pubkey,
        coin_reserve: u64,
        pc_reserve: u64,
    ) -> Self {
        Self {
            amm,
            coin_mint,
            pc_mint,
            token_coin,
            token_pc,
            amm_open_orders: Pubkey::default(),
            amm_target_orders: Pubkey::default(),
            serum_program: Pubkey::default(),
            serum_market: Pubkey::default(),
            serum_bids: Pubkey::default(),
            serum_asks: Pubkey::default(),
            serum_event_queue: Pubkey::default(),
            serum_coin_vault_account: Pubkey::default(),
            serum_pc_vault_account: Pubkey::default(),
            serum_vault_signer: Pubkey::default(),
            coin_reserve,
            pc_reserve,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_market_accounts(
        mut self,
        amm_open_orders: Pubkey,
        amm_target_orders: Pubkey,
        serum_program: Pubkey,
        serum_market: Pubkey,
        serum_bids: Pubkey,
        serum_asks: Pubkey,
        serum_event_queue: Pubkey,
        serum_coin_vault_account: Pubkey,
        serum_pc_vault_account: Pubkey,
        serum_vault_signer: Pubkey,
    ) -> Self {
        self.amm_open_orders = amm_open_orders;
        self.amm_target_orders = amm_target_orders;
        self.serum_program = serum_program;
        self.serum_market = serum_market;
        self.serum_bids = serum_bids;
        self.serum_asks = serum_asks;
        self.serum_event_queue = serum_event_queue;
        self.serum_coin_vault_account = serum_coin_vault_account;
        self.serum_pc_vault_account = serum_pc_vault_account;
        self.serum_vault_signer = serum_vault_signer;
        self
    }

    pub async fn from_amm_address_by_rpc(
        rpc: &SolanaRpcClient,
        amm: Pubkey,
    ) -> Result<Self, anyhow::Error> {
        let amm_info = crate::instruction::utils::raydium_amm_v4::fetch_amm_info(rpc, amm).await?;
        let market_state =
            crate::instruction::utils::raydium_amm_v4::fetch_market_state(rpc, amm_info.market)
                .await?;
        let serum_vault_signer =
            crate::instruction::utils::raydium_amm_v4::derive_serum_vault_signer(
                &amm_info.serum_dex,
                &amm_info.market,
                market_state.vault_signer_nonce,
            )?;
        let (coin_reserve, pc_reserve) =
            get_multi_token_balances(rpc, &amm_info.token_coin, &amm_info.token_pc).await?;
        Ok(Self {
            amm,
            coin_mint: amm_info.coin_mint,
            pc_mint: amm_info.pc_mint,
            token_coin: amm_info.token_coin,
            token_pc: amm_info.token_pc,
            amm_open_orders: amm_info.open_orders,
            amm_target_orders: amm_info.target_orders,
            serum_program: amm_info.serum_dex,
            serum_market: amm_info.market,
            serum_bids: market_state.serum_bids,
            serum_asks: market_state.serum_asks,
            serum_event_queue: market_state.serum_event_queue,
            serum_coin_vault_account: market_state.serum_coin_vault_account,
            serum_pc_vault_account: market_state.serum_pc_vault_account,
            serum_vault_signer,
            coin_reserve,
            pc_reserve,
        })
    }
}
