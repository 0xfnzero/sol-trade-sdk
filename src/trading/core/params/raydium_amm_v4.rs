use crate::common::SolanaRpcClient;
use crate::instruction::utils::raydium_amm_v4::accounts::{
    SWAP_FEE_DENOMINATOR, SWAP_FEE_NUMERATOR,
};
use crate::instruction::utils::raydium_amm_v4_types::AmmInfo;
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
    /// Swap fee taken from the input, `swap_fee_numerator / swap_fee_denominator`
    /// (the pool's `fees`; 25 / 10000 on standard pools).
    pub swap_fee_numerator: u64,
    pub swap_fee_denominator: u64,
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
            swap_fee_numerator: SWAP_FEE_NUMERATOR,
            swap_fee_denominator: SWAP_FEE_DENOMINATOR,
        }
    }

    /// The pool's own swap fee, from its `fees`.
    pub fn with_swap_fee(mut self, numerator: u64, denominator: u64) -> Self {
        self.swap_fee_numerator = numerator;
        self.swap_fee_denominator = denominator;
        self
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
        let (coin_vault, pc_vault) =
            get_multi_token_balances(rpc, &amm_info.token_coin, &amm_info.token_pc).await?;
        let (coin_reserve, pc_reserve) =
            reserves_without_take_pnl(&amm_info, coin_vault, pc_vault)?;
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
            swap_fee_numerator: amm_info.fees.swap_fee_numerator,
            swap_fee_denominator: amm_info.fees.swap_fee_denominator,
        })
    }

    /// Addresses of the accounts a quote of `amm` reads: the pool, its coin
    /// vault, its pc vault. `swap_base_in_v2` needs no market accounts.
    pub fn quote_account_keys(amm: &Pubkey, amm_info: &AmmInfo) -> Vec<Pubkey> {
        vec![*amm, amm_info.token_coin, amm_info.token_pc]
    }

    /// Params for `swap_base_in_v2` from the pool and vault accounts read
    /// together, with the pool's swap fee and its reserves net of pnl.
    pub fn from_quote_accounts(
        amm: Pubkey,
        amm_data: &[u8],
        coin_vault: &[u8],
        pc_vault: &[u8],
    ) -> Result<Self, anyhow::Error> {
        let amm_info = crate::instruction::utils::raydium_amm_v4_types::amm_info_decode(amm_data)
            .ok_or_else(|| anyhow::anyhow!("{amm} is not a Raydium AMM v4 pool"))?;
        let balance = |data: &[u8]| {
            data.get(64..72)
                .map(|raw| u64::from_le_bytes(raw.try_into().unwrap()))
                .ok_or_else(|| anyhow::anyhow!("Raydium AMM v4 vault data is too short"))
        };
        let (coin_reserve, pc_reserve) =
            reserves_without_take_pnl(&amm_info, balance(coin_vault)?, balance(pc_vault)?)?;
        Ok(Self::new(
            amm,
            amm_info.coin_mint,
            amm_info.pc_mint,
            amm_info.token_coin,
            amm_info.token_pc,
            coin_reserve,
            pc_reserve,
        )
        .with_swap_fee(amm_info.fees.swap_fee_numerator, amm_info.fees.swap_fee_denominator))
    }
}

/// The reserves the program swaps against: vault balances without the pnl it
/// has yet to take (`calc_total_without_take_pnl_no_orderbook`).
pub fn reserves_without_take_pnl(
    amm_info: &AmmInfo,
    coin_vault: u64,
    pc_vault: u64,
) -> Result<(u64, u64), anyhow::Error> {
    let coin = coin_vault.checked_sub(amm_info.out_put.need_take_pnl_coin);
    let pc = pc_vault.checked_sub(amm_info.out_put.need_take_pnl_pc);
    match (coin, pc) {
        (Some(coin), Some(pc)) => Ok((coin, pc)),
        _ => Err(anyhow::anyhow!("AMM v4 vaults hold less than the pnl the pool owes")),
    }
}
