use crate::common::bonding_curve::BondingCurveAccount;
use crate::common::spl_associated_token_account::get_associated_token_address_with_program_id;
use crate::common::SolanaRpcClient;
use crate::instruction::utils::pumpfun::reconcile_mayhem_mode_for_trade;
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;

/// PumpFun protocol specific parameters
/// Configuration parameters specific to PumpFun trading protocol.
///
/// **Creator vault**: Pump buy/sell pass `creator_vault` = `PDA(["creator-vault", authority])`.
/// Usually `authority` is [`BondingCurveAccount::creator`]; with **Creator Rewards Sharing** it is
/// `fee_sharing_config_pda(mint)` (see [`fetch_fee_sharing_creator_vault_if_active`](crate::instruction::utils::pumpfun::fetch_fee_sharing_creator_vault_if_active)).
/// **Buy/sell**：`creator_vault` 及（若可得）**`tradeEvent` / CPI 日志中的 `creator`** 优先于陈旧的曲线快照；
/// ix 组装与链下询价见 [`Self::effective_creator_for_trade`]、[`crate::instruction::utils::pumpfun::resolve_creator_vault_for_ix_with_fee_sharing`]。
///
/// **V2 instructions**: Set `use_v2_ix = true` to use `buy_v2`/`sell_v2`/`buy_exact_quote_in_v2`
/// with unified 27/26-account layout. Required for USDC-paired coins (`quote_mint != WSOL`).
/// For SOL-paired coins, legacy instructions still work and are the default.
#[derive(Clone)]
pub struct PumpFunParams {
    pub bonding_curve: Arc<BondingCurveAccount>,
    pub associated_bonding_curve: Pubkey,
    /// 最新一笔可观测 trade 的 **`tradeEvent.creator`（日志）**。当 `Some` 且非 default 时，
    /// **优先于** `bonding_curve.creator` 用于链下 creator-fee 询价与 `creator_vault` 在缺省 ix 时的推导。
    /// Pump 上 creator 可能随交易推进，调用方应在每次解析到带 `creator` 的 trade 后更新（如 `.with_observed_trade_creator`）。
    pub observed_trade_creator: Option<Pubkey>,
    /// Resolved by [`resolve_creator_vault_for_ix_with_fee_sharing`](crate::instruction::utils::pumpfun::resolve_creator_vault_for_ix_with_fee_sharing)：
    /// **显式 `creator_vault`（非 default、非 phantom）永远优先并按原值使用**，不会再用 `creator` 重算覆盖；
    /// 未传时再按 `fee_sharing_creator_vault_if_active`、`PDA(effective_creator)`（见 [`Self::effective_creator_for_trade`]）。
    pub creator_vault: Pubkey,
    /// `Some(PDA(["creator-vault", fee_sharing_config]))` when pump-fees `SharingConfig` is **Active**; set by `from_mint_by_rpc` / [`refresh_fee_sharing_creator_vault_from_rpc`](Self::refresh_fee_sharing_creator_vault_from_rpc).
    pub fee_sharing_creator_vault_if_active: Option<Pubkey>,
    /// SPL Token or Token-2022 program id owning the **mint** (from gRPC / parser / cache).
    /// **`Pubkey::default()`**：ix 构建时使用 SDK 默认 **Token-2022**（与多数 Pump.fun 新发一致）。
    /// 显式传入 Legacy 或 Token-2022 id 时严格按该值组装，不再用 mint 字符串后缀猜测。
    pub token_program: Pubkey,
    /// Whether to close token account when selling, only effective during sell operations
    pub close_token_account_when_sell: Option<bool>,
    /// Fee recipient for buy/sell account #2. Set from sol-parser-sdk (`tradeEvent.feeRecipient` / 同笔 create_v2+buy 回填的 `observed_fee_recipient`)；热路径不查 RPC。
    /// `Pubkey::default()` 时只能使用 SDK 静态 fallback，可能落后于主网 Global；交易热路径应优先传入 gRPC / parser 观测值。
    pub fee_recipient: Pubkey,
    /// Quote mint for v2 instructions (default: `So11111111111111111111111111111111111111112` for SOL-paired).
    /// For USDC-paired coins, set to `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v`.
    pub quote_mint: Pubkey,
    /// Whether to use v2 instructions (`buy_v2`/`sell_v2`/`buy_exact_quote_in_v2`).
    /// Default `false` for backward compatibility. Must be `true` for USDC-paired coins.
    pub use_v2_ix: bool,
}

impl PumpFunParams {
    pub fn immediate_sell(
        creator_vault: Pubkey,
        token_program: Pubkey,
        close_token_account_when_sell: bool,
    ) -> Self {
        Self {
            bonding_curve: Arc::new(BondingCurveAccount { ..Default::default() }),
            associated_bonding_curve: Pubkey::default(),
            observed_trade_creator: None,
            creator_vault: creator_vault,
            fee_sharing_creator_vault_if_active: None,
            token_program: token_program,
            close_token_account_when_sell: Some(close_token_account_when_sell),
            fee_recipient: Pubkey::default(),
            quote_mint: Pubkey::default(),
            use_v2_ix: false,
        }
    }

    /// When building from event/parser (e.g. sol-parser-sdk), pass `is_cashback_coin` from the event
    /// so that sell instructions include the correct remaining accounts for cashback.
    /// `mayhem_mode`: `Some` when known from Create/Trade event (`is_mayhem_mode` / `mayhem_mode`).
    /// `None` falls back to detecting Mayhem via reserved fee recipient pubkeys only (not AMM protocol fee accounts).
    pub fn from_dev_trade(
        mint: Pubkey,
        token_amount: u64,
        max_sol_cost: u64,
        creator: Pubkey,
        bonding_curve: Pubkey,
        associated_bonding_curve: Pubkey,
        creator_vault: Pubkey,
        close_token_account_when_sell: Option<bool>,
        fee_recipient: Pubkey,
        token_program: Pubkey,
        is_cashback_coin: bool,
        mayhem_mode: Option<bool>,
    ) -> Self {
        let is_mayhem_mode = reconcile_mayhem_mode_for_trade(mayhem_mode, &fee_recipient);
        let bonding_curve_account = BondingCurveAccount::from_dev_trade(
            bonding_curve,
            &mint,
            token_amount,
            max_sol_cost,
            creator,
            is_mayhem_mode,
            is_cashback_coin,
        );
        let creator_vault_resolved =
            crate::instruction::utils::pumpfun::resolve_creator_vault_for_ix_with_fee_sharing(
                &bonding_curve_account.creator,
                creator_vault,
                &mint,
                None,
            )
            .or_else(|| {
                crate::instruction::utils::pumpfun::get_creator_vault_pda(
                    &bonding_curve_account.creator,
                )
            })
            .unwrap_or_default();
        Self {
            bonding_curve: Arc::new(bonding_curve_account),
            associated_bonding_curve: associated_bonding_curve,
            observed_trade_creator: (creator != Pubkey::default()).then_some(creator),
            creator_vault: creator_vault_resolved,
            fee_sharing_creator_vault_if_active: None,
            close_token_account_when_sell: close_token_account_when_sell,
            token_program: token_program,
            fee_recipient,
            quote_mint: Pubkey::default(),
            use_v2_ix: false,
        }
    }

    /// When building from event/parser (e.g. sol-parser-sdk), pass `is_cashback_coin` from the event
    /// so that sell instructions include the correct remaining accounts for cashback.
    ///
    /// `mayhem_mode`:
    /// - **`Some(v)`**：优先采用 gRPC / `tradeEvent`，但与 **`fee_recipient` 所属池**（Mayhem vs 普通，见 pump-public-docs）不一致时，以 fee 地址为准纠偏，避免链上 `NotAuthorized`。
    /// - **`None`**：用 `fee_recipient` 是否落在 Mayhem 静态列表推断。
    pub fn from_trade(
        bonding_curve: Pubkey,
        associated_bonding_curve: Pubkey,
        mint: Pubkey,
        creator: Pubkey,
        creator_vault: Pubkey,
        virtual_token_reserves: u64,
        virtual_sol_reserves: u64,
        real_token_reserves: u64,
        real_sol_reserves: u64,
        close_token_account_when_sell: Option<bool>,
        fee_recipient: Pubkey,
        token_program: Pubkey,
        is_cashback_coin: bool,
        mayhem_mode: Option<bool>,
    ) -> Self {
        let is_mayhem_mode = reconcile_mayhem_mode_for_trade(mayhem_mode, &fee_recipient);
        let bonding_curve = BondingCurveAccount::from_trade(
            bonding_curve,
            mint,
            creator,
            virtual_token_reserves,
            virtual_sol_reserves,
            real_token_reserves,
            real_sol_reserves,
            is_mayhem_mode,
            is_cashback_coin,
        );
        let creator_vault_resolved =
            crate::instruction::utils::pumpfun::resolve_creator_vault_for_ix_with_fee_sharing(
                &bonding_curve.creator,
                creator_vault,
                &mint,
                None,
            )
            .or_else(|| {
                crate::instruction::utils::pumpfun::get_creator_vault_pda(&bonding_curve.creator)
            })
            .unwrap_or_default();
        Self {
            bonding_curve: Arc::new(bonding_curve),
            associated_bonding_curve: associated_bonding_curve,
            observed_trade_creator: (creator != Pubkey::default()).then_some(creator),
            creator_vault: creator_vault_resolved,
            fee_sharing_creator_vault_if_active: None,
            close_token_account_when_sell: close_token_account_when_sell,
            token_program: token_program,
            fee_recipient,
            quote_mint: Pubkey::default(),
            use_v2_ix: false,
        }
    }

    /// 仅 RPC 读取曲线快照；[`Self::observed_trade_creator`] 为 `None`，便于 bot 缓存合并时用粘性的 trade 日志 creator 覆盖陈旧曲线推导。
    pub async fn from_mint_by_rpc(
        rpc: &SolanaRpcClient,
        mint: &Pubkey,
    ) -> Result<Self, anyhow::Error> {
        let account =
            crate::instruction::utils::pumpfun::fetch_bonding_curve_account(rpc, mint).await?;
        let mint_account = rpc.get_account(&mint).await?;
        let bonding_curve = BondingCurveAccount {
            discriminator: 0,
            account: account.1,
            virtual_token_reserves: account.0.virtual_token_reserves,
            virtual_sol_reserves: account.0.virtual_sol_reserves,
            real_token_reserves: account.0.real_token_reserves,
            real_sol_reserves: account.0.real_sol_reserves,
            token_total_supply: account.0.token_total_supply,
            complete: account.0.complete,
            creator: account.0.creator,
            is_mayhem_mode: account.0.is_mayhem_mode,
            is_cashback_coin: account.0.is_cashback_coin,
        };
        let associated_bonding_curve = get_associated_token_address_with_program_id(
            &bonding_curve.account,
            mint,
            &mint_account.owner,
        );
        let fee_sharing_creator_vault_if_active =
            crate::instruction::utils::pumpfun::fetch_fee_sharing_creator_vault_if_active(
                rpc, mint,
            )
            .await?;
        let creator_vault =
            crate::instruction::utils::pumpfun::resolve_creator_vault_for_ix_with_fee_sharing(
                &bonding_curve.creator,
                Pubkey::default(),
                mint,
                fee_sharing_creator_vault_if_active,
            )
            .or_else(|| {
                crate::instruction::utils::pumpfun::get_creator_vault_pda(&bonding_curve.creator)
            })
            .unwrap_or_default();
        Ok(Self {
            bonding_curve: Arc::new(bonding_curve),
            associated_bonding_curve: associated_bonding_curve,
            observed_trade_creator: None,
            creator_vault,
            fee_sharing_creator_vault_if_active,
            close_token_account_when_sell: None,
            token_program: mint_account.owner,
            fee_recipient: Pubkey::default(),
            quote_mint: Pubkey::default(),
            use_v2_ix: false,
        })
    }

    /// 链下公式与 **`creator_vault` 推导回退**：**日志/事件 `creator`**（若已写入 `observed_trade_creator`）
    /// 优先，否则使用 `bonding_curve.creator`。
    #[inline]
    pub fn effective_creator_for_trade(&self) -> Pubkey {
        self.observed_trade_creator
            .filter(|c| *c != Pubkey::default())
            .unwrap_or(self.bonding_curve.creator)
    }

    /// One `getAccount` on pump-fees `SharingConfig` + re-resolves [`Self::creator_vault`]. Call before sell
    /// when params come from gRPC/cache so migrated fee-sharing mints do not hit Anchor 2006.
    pub async fn refresh_fee_sharing_creator_vault_from_rpc(
        mut self,
        rpc: &SolanaRpcClient,
        mint: &Pubkey,
    ) -> Result<Self, anyhow::Error> {
        self.fee_sharing_creator_vault_if_active =
            crate::instruction::utils::pumpfun::fetch_fee_sharing_creator_vault_if_active(
                rpc, mint,
            )
            .await?;
        let c = self.effective_creator_for_trade();
        if let Some(v) =
            crate::instruction::utils::pumpfun::resolve_creator_vault_for_ix_with_fee_sharing(
                &c,
                self.creator_vault,
                mint,
                self.fee_sharing_creator_vault_if_active,
            )
        {
            self.creator_vault = v;
        }
        Ok(self)
    }

    /// Sets `quote_mint` and enables v2 instructions. Required for USDC-paired coins.
    /// For SOL-paired coins, pass `WSOL_TOKEN_ACCOUNT` or leave default.
    #[inline]
    pub fn with_quote_mint(mut self, quote_mint: Pubkey) -> Self {
        self.quote_mint = quote_mint;
        self.use_v2_ix = quote_mint != Pubkey::default();
        self
    }

    /// Updates the cached `creator_vault` field only. Buy/sell ix use [`Self::effective_creator_for_trade`] + resolve.
    #[inline]
    pub fn with_creator_vault(mut self, creator_vault: Pubkey) -> Self {
        self.creator_vault = creator_vault;
        self
    }

    /// 覆盖 **最新一笔 trade 日志中的 `creator`**（`tradeEvent.creator`）。`None` 或 default 会清除覆盖。
    #[inline]
    pub fn with_observed_trade_creator(mut self, c: Option<Pubkey>) -> Self {
        self.observed_trade_creator = c.filter(|x| *x != Pubkey::default());
        self
    }

    /// Override fee-sharing vault hint (e.g. from an off-chain indexer). `None` clears the hint.
    #[inline]
    pub fn with_fee_sharing_creator_vault_if_active(
        mut self,
        fee_sharing_creator_vault_if_active: Option<Pubkey>,
    ) -> Self {
        self.fee_sharing_creator_vault_if_active = fee_sharing_creator_vault_if_active;
        self
    }
}
