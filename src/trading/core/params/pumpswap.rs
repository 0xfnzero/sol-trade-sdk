use crate::common::SolanaRpcClient;
use crate::instruction::utils::pumpswap::{
    accounts::MAYHEM_FEE_RECIPIENT as MAYHEM_FEE_RECIPIENT_SWAP, PumpSwapFeeBasisPoints,
};
use solana_sdk::pubkey::Pubkey;

/// PumpSwap Protocol Specific Parameters
///
/// Parameters for configuring PumpSwap trading protocol, including liquidity pool information,
/// token configuration, and transaction amounts.
///
/// **Performance Note**: If these parameters are not provided, the system will attempt to
/// retrieve the relevant information from RPC, which will increase transaction time.
/// For optimal performance, it is recommended to provide all necessary parameters in advance.
#[derive(Clone)]
pub struct PumpSwapParams {
    /// Liquidity pool address
    pub pool: Pubkey,
    /// Base token mint address
    /// The mint account address of the base token in the trading pair
    pub base_mint: Pubkey,
    /// Quote token mint address
    /// The mint account address of the quote token in the trading pair, usually SOL or USDC
    pub quote_mint: Pubkey,
    /// Pool base token account
    pub pool_base_token_account: Pubkey,
    /// Pool quote token account
    pub pool_quote_token_account: Pubkey,
    /// Base token reserves in the pool
    pub pool_base_token_reserves: u64,
    /// Raw quote-vault token balance. Pricing uses this plus [`Self::virtual_quote_reserves`].
    pub pool_quote_token_reserves: u64,
    /// Signed virtual quote reserves from the PumpSwap Pool account or trade event.
    pub virtual_quote_reserves: i128,
    /// Coin creator vault ATA
    pub coin_creator_vault_ata: Pubkey,
    /// Coin creator vault authority
    pub coin_creator_vault_authority: Pubkey,
    /// Token program ID
    pub base_token_program: Pubkey,
    /// Quote token program ID
    pub quote_token_program: Pubkey,
    /// Whether the pool is in mayhem mode
    pub is_mayhem_mode: bool,
    /// Pool creator. Canonical PumpSwap pools use the Pump program pool-authority PDA here;
    /// fee tiers are selected from this value without doing RPC in the instruction builder.
    pub pool_creator: Pubkey,
    /// Pool [`Pool::coin_creator`](crate::instruction::utils::pumpswap_types::Pool). Used for PumpSwap
    /// `remaining_accounts`: **`pool-v2` is appended only when this is not `Pubkey::default()`
    /// (matches `@pump-fun/pump-swap-sdk`); wrong flag causes buys to revert with buyback recipient errors (e.g. 6053).
    pub coin_creator: Pubkey,
    /// Whether the pool's coin has cashback enabled
    pub is_cashback_coin: bool,
    /// Cashback fee in basis points (from trade events / sol-parser-sdk). For quote-in buy and base-in sell
    /// math, this is summed with [`COIN_CREATOR_FEE_BASIS_POINTS`](crate::instruction::utils::pumpswap::accounts::COIN_CREATOR_FEE_BASIS_POINTS)
    /// when a creator vault applies — matching on-chain treating creator + cashback as one fee bucket.
    /// Use `0` when unknown (e.g. RPC-only pool decode has no per-mint cashback bps).
    pub cashback_fee_basis_points: u64,
    /// Base mint supply used by PumpSwap fee-tier market-cap selection. Filled by RPC
    /// constructors and optional for parser/event fast paths.
    pub base_mint_supply: Option<u64>,
    /// Effective PumpSwap fee bps for this pool snapshot. Instruction building reads this
    /// only from params, so hot-path trading never adds an RPC call for fee discovery.
    pub fee_basis_points: PumpSwapFeeBasisPoints,
    /// Optional fixed protocol fee recipient (including mayhem). When set, buy/sell skip the
    /// random GlobalConfig pick — useful for deterministic tests and for sharing the same
    /// fee ATA across buy+sell so combined simulate txs stay under the size cap.
    pub protocol_fee_recipient_override: Option<Pubkey>,
    /// Optional fixed buyback / protocol-extra fee recipient (trailing remaining account).
    pub protocol_extra_fee_recipient_override: Option<Pubkey>,
}

impl PumpSwapParams {
    pub fn new(
        pool: Pubkey,
        base_mint: Pubkey,
        quote_mint: Pubkey,
        pool_base_token_account: Pubkey,
        pool_quote_token_account: Pubkey,
        pool_base_token_reserves: u64,
        pool_quote_token_reserves: u64,
        virtual_quote_reserves: i128,
        coin_creator_vault_ata: Pubkey,
        coin_creator_vault_authority: Pubkey,
        base_token_program: Pubkey,
        quote_token_program: Pubkey,
        fee_recipient: Pubkey,
        coin_creator: Pubkey,
        is_cashback_coin: bool,
        cashback_fee_basis_points: u64,
    ) -> Self {
        let is_mayhem_mode = fee_recipient == MAYHEM_FEE_RECIPIENT_SWAP;
        let creator_fee_basis_points = if coin_creator == Pubkey::default() {
            0
        } else {
            crate::instruction::utils::pumpswap::accounts::COIN_CREATOR_FEE_BASIS_POINTS
        }
        .saturating_add(cashback_fee_basis_points);
        Self {
            pool,
            base_mint,
            quote_mint,
            pool_base_token_account,
            pool_quote_token_account,
            pool_base_token_reserves,
            pool_quote_token_reserves,
            virtual_quote_reserves,
            coin_creator_vault_ata,
            coin_creator_vault_authority,
            base_token_program,
            quote_token_program,
            is_mayhem_mode,
            pool_creator: Pubkey::default(),
            coin_creator,
            is_cashback_coin,
            cashback_fee_basis_points,
            base_mint_supply: None,
            fee_basis_points: PumpSwapFeeBasisPoints::new(
                crate::instruction::utils::pumpswap::accounts::LP_FEE_BASIS_POINTS,
                crate::instruction::utils::pumpswap::accounts::PROTOCOL_FEE_BASIS_POINTS,
                creator_fee_basis_points,
            ),
            protocol_fee_recipient_override: None,
            protocol_extra_fee_recipient_override: None,
        }
    }

    pub fn with_pool_creator(mut self, pool_creator: Pubkey) -> Self {
        self.pool_creator = pool_creator;
        self
    }

    /// Pin protocol + buyback fee recipients so buy/sell share the same fee ATAs.
    pub fn with_fee_recipients(
        mut self,
        protocol_fee_recipient: Pubkey,
        protocol_extra_fee_recipient: Pubkey,
    ) -> Self {
        self.protocol_fee_recipient_override = Some(protocol_fee_recipient);
        self.protocol_extra_fee_recipient_override = Some(protocol_extra_fee_recipient);
        self
    }

    pub fn with_base_mint_supply(mut self, base_mint_supply: u64) -> Self {
        self.base_mint_supply = Some(base_mint_supply);
        self
    }

    /// Quote reserves used by PumpSwap pricing and fee-tier selection.
    pub fn effective_quote_reserves(&self) -> Result<u64, anyhow::Error> {
        crate::instruction::utils::pumpswap_types::effective_quote_reserves(
            self.pool_quote_token_reserves,
            self.virtual_quote_reserves,
        )
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Invalid PumpSwap effective quote reserves: vault={} virtual={}",
                self.pool_quote_token_reserves,
                self.virtual_quote_reserves
            )
        })
    }

    pub fn with_fee_basis_points(
        mut self,
        lp_fee_basis_points: u64,
        protocol_fee_basis_points: u64,
        coin_creator_fee_basis_points: u64,
    ) -> Self {
        let creator_fee_basis_points =
            if self.coin_creator == Pubkey::default() { 0 } else { coin_creator_fee_basis_points }
                .saturating_add(self.cashback_fee_basis_points);
        self.fee_basis_points = PumpSwapFeeBasisPoints::new(
            lp_fee_basis_points,
            protocol_fee_basis_points,
            creator_fee_basis_points,
        );
        self
    }

    /// Replace the cashback rate while preserving the configured creator rate.
    /// Repeated calls replace the old cashback component rather than adding it again.
    /// The cashback account flag is independent and remains sourced from Pool/events.
    pub fn with_cashback_fee_basis_points(
        mut self,
        cashback_fee_basis_points: u64,
    ) -> Result<Self, anyhow::Error> {
        let creator_rate = self
            .fee_basis_points
            .coin_creator_fee_basis_points
            .checked_sub(self.cashback_fee_basis_points)
            .ok_or_else(|| anyhow::anyhow!("Inconsistent PumpSwap cashback fee component"))?;
        let combined_rate = creator_rate
            .checked_add(cashback_fee_basis_points)
            .ok_or_else(|| anyhow::anyhow!("PumpSwap creator and cashback fee overflow"))?;
        self.cashback_fee_basis_points = cashback_fee_basis_points;
        self.fee_basis_points.coin_creator_fee_basis_points = combined_rate;
        Ok(self)
    }

    /// Fast-path constructor for building PumpSwap parameters directly from decoded
    /// trade/event data and the accompanying instruction accounts, avoiding RPC
    /// lookups and associated latency. Token program IDs should be sourced from
    /// the instruction accounts themselves to respect Token Program vs Token-2022
    /// differences.
    ///
    /// When building from event/parser (e.g. sol-parser-sdk), pass `is_cashback_coin`
    /// from the event so that buy/sell instructions include the correct remaining
    /// accounts for cashback.
    /// This constructor retains legacy default fees. For current fee schedules use
    /// `from_trade_with_fee_basis_points`, or set fees computed from cached Pool,
    /// FeeConfig and GlobalConfig via `compute_fee_basis_points_for_pool`.
    /// Raw shred outer instructions do not contain executed reserves or fee bps;
    /// populate those inputs from a coherent account cache before quoting.
    pub fn from_trade(
        pool: Pubkey,
        base_mint: Pubkey,
        quote_mint: Pubkey,
        pool_base_token_account: Pubkey,
        pool_quote_token_account: Pubkey,
        pool_base_token_reserves: u64,
        pool_quote_token_reserves: u64,
        virtual_quote_reserves: i128,
        coin_creator_vault_ata: Pubkey,
        coin_creator_vault_authority: Pubkey,
        base_token_program: Pubkey,
        quote_token_program: Pubkey,
        fee_recipient: Pubkey,
        coin_creator: Pubkey,
        is_cashback_coin: bool,
        cashback_fee_basis_points: u64,
    ) -> Self {
        Self::new(
            pool,
            base_mint,
            quote_mint,
            pool_base_token_account,
            pool_quote_token_account,
            pool_base_token_reserves,
            pool_quote_token_reserves,
            virtual_quote_reserves,
            coin_creator_vault_ata,
            coin_creator_vault_authority,
            base_token_program,
            quote_token_program,
            fee_recipient,
            coin_creator,
            is_cashback_coin,
            cashback_fee_basis_points,
        )
    }

    /// Fast-path constructor for parser/event feeds that already include fee bps.
    ///
    /// This avoids any fee-discovery RPC and is the preferred path when sol-parser-sdk or
    /// another stream parser provides `lp_fee_basis_points`, `protocol_fee_basis_points`, and
    /// `coin_creator_fee_basis_points` from PumpSwap events.
    pub fn from_trade_with_fee_basis_points(
        pool: Pubkey,
        base_mint: Pubkey,
        quote_mint: Pubkey,
        pool_base_token_account: Pubkey,
        pool_quote_token_account: Pubkey,
        pool_base_token_reserves: u64,
        pool_quote_token_reserves: u64,
        virtual_quote_reserves: i128,
        coin_creator_vault_ata: Pubkey,
        coin_creator_vault_authority: Pubkey,
        base_token_program: Pubkey,
        quote_token_program: Pubkey,
        fee_recipient: Pubkey,
        pool_creator: Pubkey,
        coin_creator: Pubkey,
        is_cashback_coin: bool,
        cashback_fee_basis_points: u64,
        lp_fee_basis_points: u64,
        protocol_fee_basis_points: u64,
        coin_creator_fee_basis_points: u64,
    ) -> Self {
        Self::new(
            pool,
            base_mint,
            quote_mint,
            pool_base_token_account,
            pool_quote_token_account,
            pool_base_token_reserves,
            pool_quote_token_reserves,
            virtual_quote_reserves,
            coin_creator_vault_ata,
            coin_creator_vault_authority,
            base_token_program,
            quote_token_program,
            fee_recipient,
            coin_creator,
            is_cashback_coin,
            cashback_fee_basis_points,
        )
        .with_pool_creator(pool_creator)
        .with_fee_basis_points(
            lp_fee_basis_points,
            protocol_fee_basis_points,
            coin_creator_fee_basis_points,
        )
    }

    pub async fn from_mint_by_rpc(
        rpc: &SolanaRpcClient,
        mint: &Pubkey,
    ) -> Result<Self, anyhow::Error> {
        let (pool_address, pool) =
            crate::instruction::utils::pumpswap::find_by_mint(rpc, mint).await?;
        Self::from_pool_data(rpc, &pool_address, &pool).await
    }

    pub async fn from_pool_address_by_rpc(
        rpc: &SolanaRpcClient,
        pool_address: &Pubkey,
    ) -> Result<Self, anyhow::Error> {
        let pool_data = crate::instruction::utils::pumpswap::fetch_pool(rpc, pool_address).await?;
        Self::from_pool_data(rpc, pool_address, &pool_data).await
    }

    /// Build params using an already-decoded Pool as an address hint.
    ///
    /// Refreshes the Pool, both vaults and base mint in one RPC snapshot so a
    /// changing virtual reserve cannot be combined with balances from another slot.
    /// The hint avoids a separate Pool lookup before this batched read.
    pub async fn from_pool_data(
        rpc: &SolanaRpcClient,
        pool_address: &Pubkey,
        pool_data: &crate::instruction::utils::pumpswap_types::Pool,
    ) -> Result<Self, anyhow::Error> {
        let (pool_data, snapshot) =
            crate::instruction::utils::pumpswap::get_pool_rpc_snapshot_with_address(
                pool_address,
                pool_data,
                rpc,
            )
            .await?;
        let (fee_config, global_config) = tokio::join!(
            crate::instruction::utils::pumpswap::fetch_fee_config(rpc),
            crate::instruction::utils::pumpswap::fetch_global_config(rpc),
        );
        Self::from_cached_pool_snapshot(
            pool_address,
            &pool_data,
            &snapshot,
            fee_config.as_ref(),
            global_config.as_ref(),
        )
    }

    /// Build parameters without legacy fee or recipient fallbacks and without RPC.
    /// Both current configurations are required. Cache account identity and freshness
    /// must still be validated by the caller. Use this for current ShredStream quotes.
    pub fn from_cached_pool_snapshot_strict(
        pool_address: &Pubkey,
        pool_data: &crate::instruction::utils::pumpswap_types::Pool,
        snapshot: &crate::instruction::utils::pumpswap::PoolRpcSnapshot,
        fee_config: &crate::instruction::utils::pumpswap::PumpSwapFeeConfig,
        global_config: &crate::instruction::utils::pumpswap::GlobalConfig,
    ) -> Result<Self, anyhow::Error> {
        for token_program in [snapshot.base_token_program, snapshot.quote_token_program] {
            if token_program != crate::constants::TOKEN_PROGRAM
                && token_program != crate::constants::TOKEN_PROGRAM_2022
            {
                return Err(anyhow::anyhow!("Unsupported PumpSwap cached token program"));
            }
        }
        let params = Self::from_cached_pool_snapshot(
            pool_address,
            pool_data,
            snapshot,
            Some(fee_config),
            Some(global_config),
        )?;
        if params.pool_quote_token_reserves == 0 || params.effective_quote_reserves()? == 0 {
            return Err(anyhow::anyhow!("PumpSwap quote reserves cannot be zero for trading"));
        }
        if params.protocol_fee_recipient_override.is_none() {
            return Err(anyhow::anyhow!(
                "PumpSwap config has no usable protocol/mayhem fee recipient"
            ));
        }
        if params.protocol_extra_fee_recipient_override.is_none() {
            return Err(anyhow::anyhow!("PumpSwap config has no usable buyback fee recipient"));
        }
        Ok(params)
    }

    /// Build current PumpSwap parameters without RPC from caller-maintained state.
    /// Pool and vault balances must belong to a coherent snapshot. Configurations
    /// must be refreshed by the caller when they change. This does not validate
    /// account addresses or freshness; validate them when populating the cache.
    /// Cashback bps are not present in Pool; supply them separately when needed.
    /// Missing configurations or empty recipient sets retain legacy fallbacks;
    /// prefer `from_cached_pool_snapshot_strict` for current account-cache trading.
    pub fn from_cached_pool_snapshot(
        pool_address: &Pubkey,
        pool_data: &crate::instruction::utils::pumpswap_types::Pool,
        snapshot: &crate::instruction::utils::pumpswap::PoolRpcSnapshot,
        fee_config: Option<&crate::instruction::utils::pumpswap::PumpSwapFeeConfig>,
        global_config: Option<&crate::instruction::utils::pumpswap::GlobalConfig>,
    ) -> Result<Self, anyhow::Error> {
        let pool_base_token_reserves = snapshot.base_reserve;
        let pool_quote_token_reserves = snapshot.quote_reserve;
        let effective_quote_token_reserves =
            crate::instruction::utils::pumpswap_types::effective_quote_reserves(
                pool_quote_token_reserves,
                pool_data.virtual_quote_reserves,
            )
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Invalid PumpSwap effective quote reserves: vault={} virtual={}",
                    pool_quote_token_reserves,
                    pool_data.virtual_quote_reserves
                )
            })?;
        let base_mint_supply = Some(snapshot.base_mint_supply);
        let raw_fee_basis_points =
            crate::instruction::utils::pumpswap::try_compute_fee_basis_points_for_pool(
                fee_config,
                global_config,
                pool_data,
                snapshot.base_mint_supply,
                pool_base_token_reserves,
                effective_quote_token_reserves,
            )?;
        let creator_fee_basis_points = if pool_data.coin_creator == Pubkey::default() {
            0
        } else {
            raw_fee_basis_points.coin_creator_fee_basis_points
        };
        let creator = pool_data.coin_creator;
        let coin_creator_vault_ata = crate::instruction::utils::pumpswap::coin_creator_vault_ata(
            creator,
            pool_data.quote_mint,
            snapshot.quote_token_program,
        );
        let coin_creator_vault_authority =
            crate::instruction::utils::pumpswap::coin_creator_vault_authority(creator);
        let protocol_fee_recipient_override = global_config.and_then(|global| {
            let mut mayhem_recipients = [Pubkey::default(); 8];
            mayhem_recipients[0] = global.reserved_fee_recipient;
            mayhem_recipients[1..].copy_from_slice(&global.reserved_fee_recipients);
            crate::instruction::utils::pumpswap::choose_nonzero(if pool_data.is_mayhem_mode {
                &mayhem_recipients
            } else {
                &global.protocol_fee_recipients
            })
        });
        let protocol_extra_fee_recipient_override = global_config.and_then(|global| {
            crate::instruction::utils::pumpswap::choose_nonzero(&global.buyback_fee_recipients)
        });

        Ok(Self {
            pool: *pool_address,
            base_mint: pool_data.base_mint,
            quote_mint: pool_data.quote_mint,
            pool_base_token_account: pool_data.pool_base_token_account,
            pool_quote_token_account: pool_data.pool_quote_token_account,
            pool_base_token_reserves,
            pool_quote_token_reserves,
            virtual_quote_reserves: pool_data.virtual_quote_reserves,
            coin_creator_vault_ata,
            coin_creator_vault_authority,
            base_token_program: snapshot.base_token_program,
            is_cashback_coin: pool_data.is_cashback_coin,
            quote_token_program: snapshot.quote_token_program,
            is_mayhem_mode: pool_data.is_mayhem_mode,
            pool_creator: pool_data.creator,
            coin_creator: pool_data.coin_creator,
            cashback_fee_basis_points: 0,
            base_mint_supply,
            fee_basis_points: PumpSwapFeeBasisPoints::new(
                raw_fee_basis_points.lp_fee_basis_points,
                raw_fee_basis_points.protocol_fee_basis_points,
                creator_fee_basis_points,
            ),
            protocol_fee_recipient_override,
            protocol_extra_fee_recipient_override,
        })
    }
}
