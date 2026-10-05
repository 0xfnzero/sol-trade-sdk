//! Subscription-fed preparation. No RPC client or asynchronous fallback exists here.
use super::raydium_cpmm::token_transfer_fee_from_config;
use super::{BonkParams, RaydiumCpmmParams};
use crate::instruction::utils::{bonk, bonk_types, raydium_cpmm, raydium_cpmm_types};
use anyhow::{anyhow, ensure, Result};
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedAccount {
    pub owner: Pubkey,
    pub data: Vec<u8>,
    pub slot: u64,
    pub write_version: u64,
}

/// Supplied by the caller's slot/epoch subscription. Never inferred from a trade.
#[derive(Clone, Copy, Debug)]
pub struct CacheReadContext {
    pub slot: u64,
    pub epoch: u64,
    pub maximum_slot_age: u64,
}

/// Identity clues only: amounts, wallets and historical balances are not copied.
#[derive(Clone, Copy, Debug)]
pub struct PoolTradeHint {
    pub pool: Pubkey,
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
}

impl PoolTradeHint {
    #[cfg(feature = "parser-adapter")]
    pub fn from_route_leg(leg: &sol_parser_sdk::transaction_route::RouteSwapLeg) -> Result<Self> {
        ensure!(
            matches!(
                leg.protocol,
                sol_parser_sdk::transaction_route::SwapProtocol::LaunchLab
                    | sol_parser_sdk::transaction_route::SwapProtocol::RaydiumCpmm
                    | sol_parser_sdk::transaction_route::SwapProtocol::RaydiumClmm
                    | sol_parser_sdk::transaction_route::SwapProtocol::OrcaWhirlpool
                    | sol_parser_sdk::transaction_route::SwapProtocol::MeteoraDlmm
                    | sol_parser_sdk::transaction_route::SwapProtocol::RaydiumAmmV4
            ),
            "Cache adapter supports LaunchLab, CPMM, CLMM, Whirlpool and DLMM legs"
        );
        Ok(Self {
            pool: leg.pool,
            input_mint: leg.input_mint.ok_or_else(|| anyhow!("Parser input mint is unresolved"))?,
            output_mint: leg
                .output_mint
                .ok_or_else(|| anyhow!("Parser output mint is unresolved"))?,
        })
    }

    pub(super) fn matches(&self, base: Pubkey, quote: Pubkey) -> Result<()> {
        ensure!(
            base != quote && base != Pubkey::default() && quote != Pubkey::default(),
            "Invalid cached mint pair"
        );
        ensure!(
            (self.input_mint == base && self.output_mint == quote)
                || (self.input_mint == quote && self.output_mint == base),
            "Parser mint pair does not match cached pool"
        );
        Ok(())
    }
}

/// Update off the trading path and expose a stable snapshot to readers. Reorgs
/// require replacing this cache with a snapshot from the caller's chosen fork.
#[derive(Clone, Default)]
pub struct SubscriptionAccountCache {
    pub(super) accounts: HashMap<Pubkey, CachedAccount>,
}
impl SubscriptionAccountCache {
    /// Consume a raw gRPC event without copying its account-data buffer. Use the
    /// borrowing adapter when the caller still needs to retain the original event.
    #[cfg(feature = "parser-adapter")]
    pub fn update_from_owned_parser_snapshot(
        &mut self,
        event: sol_parser_sdk::accounts::liquidity_snapshot::RawAccountSnapshotEvent,
    ) -> Result<bool> {
        let account = event.account;
        self.update(
            account.pubkey,
            CachedAccount {
                owner: account.owner,
                data: if account.lamports == 0 { Vec::new() } else { account.data },
                slot: event.metadata.slot,
                write_version: event.write_version,
            },
        )
    }
    #[cfg(feature = "parser-adapter")]
    pub fn update_from_parser_snapshot(
        &mut self,
        event: &sol_parser_sdk::accounts::liquidity_snapshot::RawAccountSnapshotEvent,
    ) -> Result<bool> {
        self.update_from_parser_account(&event.account, event.metadata.slot, event.write_version)
    }
    /// Copy a parser subscription account update into the background cache.
    /// The stream's slot/write_version must be supplied; trade amounts are not state.
    #[cfg(feature = "parser-adapter")]
    pub fn update_from_parser_account(
        &mut self,
        account: &sol_parser_sdk::accounts::AccountData,
        slot: u64,
        write_version: u64,
    ) -> Result<bool> {
        let data = if account.lamports == 0 { &[][..] } else { account.data.as_slice() };
        if !self.should_update(account.pubkey, account.owner, data, slot, write_version)? {
            return Ok(false);
        }
        self.accounts.insert(
            account.pubkey,
            CachedAccount { owner: account.owner, data: data.to_vec(), slot, write_version },
        );
        Ok(true)
    }
    /// Returns false for stale or identical replay updates; conflicting equal versions fail.
    pub fn update(&mut self, key: Pubkey, account: CachedAccount) -> Result<bool> {
        if !self.should_update(
            key,
            account.owner,
            &account.data,
            account.slot,
            account.write_version,
        )? {
            return Ok(false);
        }
        self.accounts.insert(key, account);
        Ok(true)
    }

    // Inspect borrowed bytes before allocating. The cache is exclusively borrowed
    // for ingestion, so this check and the subsequent insert cannot race.
    fn should_update(
        &self,
        key: Pubkey,
        owner: Pubkey,
        data: &[u8],
        slot: u64,
        write_version: u64,
    ) -> Result<bool> {
        if let Some(old) = self.accounts.get(&key) {
            let old_version = (old.slot, old.write_version);
            let new_version = (slot, write_version);
            if new_version < old_version {
                return Ok(false);
            }
            if new_version == old_version {
                ensure!(
                    old.owner == owner && old.data == data,
                    "Conflicting account update at identical slot/write_version: {key}"
                );
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(super) fn get(&self, key: Pubkey, owner: Pubkey, ctx: CacheReadContext) -> Result<&[u8]> {
        let a = self.accounts.get(&key).ok_or_else(|| anyhow!("Missing cached account: {key}"))?;
        ensure!(a.owner == owner, "Cached account owner mismatch: {key}");
        ensure!(
            a.slot <= ctx.slot && ctx.slot - a.slot <= ctx.maximum_slot_age,
            "Cached account is stale or ahead of snapshot: {key}"
        );
        Ok(&a.data)
    }

    pub(super) fn mint(
        &self,
        key: Pubkey,
        expected_program: Option<Pubkey>,
        ctx: CacheReadContext,
    ) -> Result<(Pubkey, super::TokenTransferFee)> {
        let a = self.accounts.get(&key).ok_or_else(|| anyhow!("Missing cached mint: {key}"))?;
        if let Some(program) = expected_program {
            ensure!(a.owner == program, "Cached mint program mismatch: {key}");
        }
        let token_2022 = Pubkey::new_from_array(spl_token_2022_interface::ID.to_bytes());
        ensure!(
            a.owner == crate::constants::TOKEN_PROGRAM || a.owner == token_2022,
            "Unsupported CPMM token program: {}",
            a.owner
        );
        let data = self.get(key, a.owner, ctx)?;
        // The classic program only accepts the fixed-size Mint layout. The
        // Token-2022 decoder also accepts extended layouts, so owner must gate
        // those bytes before decoding or interpreting their extensions.
        if a.owner == crate::constants::TOKEN_PROGRAM {
            ensure!(data.len() == 82, "Invalid classic SPL mint length: {key}");
        }
        use spl_token_2022_interface::{
            extension::{BaseStateWithExtensions, ExtensionType, StateWithExtensions},
            state::Mint,
        };
        let mint = StateWithExtensions::<Mint>::unpack(data)
            .map_err(|e| anyhow!("Invalid cached mint {key}: {e}"))?;
        if a.owner == crate::constants::TOKEN_PROGRAM {
            return Ok((a.owner, super::TokenTransferFee::default()));
        }
        let mut transfer_fee = super::TokenTransferFee::default();
        for extension in mint.get_extension_types()? {
            if extension == ExtensionType::TransferFeeConfig && a.owner == token_2022 {
                let config = mint.get_extension::<spl_token_2022_interface::extension::transfer_fee::TransferFeeConfig>()?;
                transfer_fee = token_transfer_fee_from_config(config, ctx.epoch)?;
                continue;
            }
            // Stock mints can carry these extensions while ordinary public
            // transfers remain enabled. Validate current state rather than
            // rejecting the extension's presence alone.
            if extension == ExtensionType::DefaultAccountState {
                let state = mint.get_extension::<spl_token_2022_interface::extension::default_account_state::DefaultAccountState>()?;
                ensure!(
                    state.state == spl_token_2022_interface::state::AccountState::Initialized as u8,
                    "Mint defaults to frozen/uninitialized token accounts: {key}"
                );
                continue;
            }
            if extension == ExtensionType::Pausable {
                let state = mint
                    .get_extension::<spl_token_2022_interface::extension::pausable::PausableConfig>(
                    )?;
                ensure!(!bool::from(state.paused), "Cached mint transfers are paused: {key}");
                continue;
            }
            if extension == ExtensionType::TransferHook {
                mint.get_extension::<spl_token_2022_interface::extension::transfer_hook::TransferHook>()?;
                ensure!(
                    spl_token_2022_interface::extension::transfer_hook::get_program_id(&mint)
                        .is_none(),
                    "Active transfer hook requires unsupported extra accounts: {key}"
                );
                continue;
            }
            ensure!(
                matches!(
                    extension,
                    ExtensionType::TransferFeeConfig
                        | ExtensionType::MintCloseAuthority
                        | ExtensionType::InterestBearingConfig
                        | ExtensionType::PermanentDelegate
                        | ExtensionType::MetadataPointer
                        | ExtensionType::TokenMetadata
                        | ExtensionType::GroupPointer
                        | ExtensionType::TokenGroup
                        | ExtensionType::GroupMemberPointer
                        | ExtensionType::TokenGroupMember
                        | ExtensionType::ScaledUiAmount
                        // Public transferChecked is distinct from confidential transfers.
                        | ExtensionType::ConfidentialTransferMint
                        | ExtensionType::ConfidentialTransferFeeConfig
                ),
                "Unsupported mint extension in cached swap adapter: {extension:?}"
            );
        }
        // Validate classic mints too: a missing/malformed mint must not mean zero fee.
        ensure!(data.len() >= 82 && data[45] == 1, "Invalid or uninitialized cached mint: {key}");
        Ok((a.owner, transfer_fee))
    }

    /// Builds complete LaunchLab parameters from current pool/config/mint bytes.
    pub fn launchlab(&self, hint: PoolTradeHint, ctx: CacheReadContext) -> Result<BonkParams> {
        let program = bonk::accounts::BONK;
        let data = self.get(hint.pool, program, ctx)?;
        ensure!(
            data.get(..8) == Some(&bonk_types::POOL_STATE_DISCRIMINATOR),
            "Invalid LaunchLab pool discriminator"
        );
        let p = bonk_types::pool_state_decode(&data[8..])
            .ok_or_else(|| anyhow!("Invalid cached LaunchLab pool"))?;
        hint.matches(p.base_mint, p.quote_mint)?;
        ensure!(p.status == 0, "LaunchLab pool is no longer trading; refresh migration registry");
        let fees = bonk::decode_fee_config(
            self.get(p.global_config, program, ctx)?,
            self.get(p.platform_config, program, ctx)?,
        )?;
        ensure!(fees.curve_type == 0, "Unsupported LaunchLab curve type");
        ensure!(
            fees.trade_fee_rate
                .checked_add(fees.platform_fee_rate)
                .and_then(|f| f.checked_add(fees.creator_fee_rate))
                .is_some_and(|f| f < 1_000_000),
            "Invalid LaunchLab fee rates"
        );
        let (base_program, base_fee) = self.mint(p.base_mint, None, ctx)?;
        let (quote_program, quote_fee) = self.mint(p.quote_mint, None, ctx)?;
        Ok(BonkParams {
            virtual_base: p.virtual_base as u128,
            virtual_quote: p.virtual_quote as u128,
            real_base: p.real_base as u128,
            real_quote: p.real_quote as u128,
            total_base_sell: p.total_base_sell as u128,
            pool_state: hint.pool,
            base_vault: p.base_vault,
            quote_vault: p.quote_vault,
            mint_token_program: base_program,
            quote_mint: p.quote_mint,
            quote_token_program: quote_program,
            platform_config: p.platform_config,
            global_config: p.global_config,
            platform_associated_account: bonk::get_platform_associated_account_for_quote(
                &p.platform_config,
                &p.quote_mint,
            )
            .ok_or_else(|| anyhow!("Invalid platform PDA"))?,
            creator_associated_account: bonk::get_creator_associated_account_for_quote(
                &p.creator,
                &p.quote_mint,
            )
            .ok_or_else(|| anyhow!("Invalid creator PDA"))?,
            curve_type: fees.curve_type,
            trade_fee_rate: fees.trade_fee_rate,
            platform_fee_rate: fees.platform_fee_rate,
            creator_fee_rate: fees.creator_fee_rate,
            base_transfer_fee: base_fee,
            quote_transfer_fee: quote_fee,
        })
    }

    /// Additionally checks opening time against the caller's subscribed Clock.
    pub fn cpmm_at(
        &self,
        hint: PoolTradeHint,
        ctx: CacheReadContext,
        unix_timestamp: u64,
    ) -> Result<RaydiumCpmmParams> {
        self.cpmm_impl(hint, ctx, Some(unix_timestamp))
    }

    /// Preserves token0/token1 orientation even when the observed trade was reversed.
    pub fn cpmm(&self, hint: PoolTradeHint, ctx: CacheReadContext) -> Result<RaydiumCpmmParams> {
        self.cpmm_impl(hint, ctx, None)
    }

    fn cpmm_impl(
        &self,
        hint: PoolTradeHint,
        ctx: CacheReadContext,
        unix_timestamp: Option<u64>,
    ) -> Result<RaydiumCpmmParams> {
        let program = raydium_cpmm::accounts::RAYDIUM_CPMM;
        let data = self.get(hint.pool, program, ctx)?;
        ensure!(
            data.get(..8) == Some(&raydium_cpmm_types::POOL_STATE_DISCRIMINATOR),
            "Invalid CPMM pool discriminator"
        );
        let p = raydium_cpmm_types::pool_state_decode(&data[8..])
            .ok_or_else(|| anyhow!("Invalid cached CPMM pool"))?;
        hint.matches(p.token0_mint, p.token1_mint)?;
        // Check the subscribed Clock before config/mint/vault work, using the
        // pool already decoded for preparation rather than fetching it again.
        if let Some(unix_timestamp) = unix_timestamp {
            ensure!(unix_timestamp >= p.open_time, "CPMM pool has not opened");
        }
        ensure!(p.status & 4 == 0, "CPMM swap is disabled");
        let data = self.get(p.amm_config, program, ctx)?;
        ensure!(
            data.get(..8) == Some(&raydium_cpmm_types::AMM_CONFIG_DISCRIMINATOR),
            "Invalid CPMM config discriminator"
        );
        let f = raydium_cpmm_types::amm_config_decode(&data[8..])
            .ok_or_else(|| anyhow!("Invalid cached CPMM config"))?;
        ensure!(
            f.trade_fee_rate
                .checked_add(if p.enable_creator_fee { f.creator_fee_rate } else { 0 })
                .is_some_and(|fee| fee < 1_000_000)
                && f.protocol_fee_rate
                    .checked_add(f.fund_fee_rate)
                    .is_some_and(|fee| fee <= 1_000_000)
                && p.creator_fee_on <= 2,
            "Invalid CPMM fee configuration"
        );
        let reserve = |vault: Pubkey,
                       mint: Pubkey,
                       token_program: Pubkey,
                       protocol: u64,
                       fund: u64,
                       creator: u64|
         -> Result<u64> {
            let data = self.get(vault, token_program, ctx)?;
            if token_program == crate::constants::TOKEN_PROGRAM {
                ensure!(data.len() == 165, "Invalid classic SPL vault length");
            }
            use spl_token_2022_interface::{
                extension::StateWithExtensions,
                state::{Account, AccountState},
            };
            // Borrowed decoding validates base COption fields and the extended
            // account-type marker without allocating or scanning the TLV list.
            let account = StateWithExtensions::<Account>::unpack(data)
                .map_err(|error| anyhow!("Invalid cached vault layout: {error}"))?;
            ensure!(
                account.base.mint.to_bytes() == mint.to_bytes()
                    && account.base.owner.to_bytes() == raydium_cpmm::accounts::AUTHORITY.to_bytes()
                    && account.base.state == AccountState::Initialized,
                "Invalid cached vault"
            );
            account
                .base
                .amount
                .checked_sub(protocol)
                .and_then(|a| a.checked_sub(fund))
                .and_then(|a| a.checked_sub(creator))
                .ok_or_else(|| anyhow!("CPMM fees exceed cached vault balance"))
        };
        let base_reserve = reserve(
            p.token0_vault,
            p.token0_mint,
            p.token0_program,
            p.protocol_fees_token0,
            p.fund_fees_token0,
            p.creator_fees_token0,
        )?;
        let quote_reserve = reserve(
            p.token1_vault,
            p.token1_mint,
            p.token1_program,
            p.protocol_fees_token1,
            p.fund_fees_token1,
            p.creator_fees_token1,
        )?;
        let (_, base_fee) = self.mint(p.token0_mint, Some(p.token0_program), ctx)?;
        let (_, quote_fee) = self.mint(p.token1_mint, Some(p.token1_program), ctx)?;
        Ok(RaydiumCpmmParams {
            pool_state: hint.pool,
            amm_config: p.amm_config,
            base_mint: p.token0_mint,
            quote_mint: p.token1_mint,
            base_reserve,
            quote_reserve,
            base_vault: p.token0_vault,
            quote_vault: p.token1_vault,
            base_token_program: p.token0_program,
            quote_token_program: p.token1_program,
            observation_state: p.observation_key,
            trade_fee_rate: f.trade_fee_rate,
            protocol_fee_rate: f.protocol_fee_rate,
            fund_fee_rate: f.fund_fee_rate,
            creator_fee_rate: f.creator_fee_rate,
            creator_fee_on: p.creator_fee_on,
            enable_creator_fee: p.enable_creator_fee,
            base_transfer_fee: base_fee,
            quote_transfer_fee: quote_fee,
        })
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    fn ctx() -> CacheReadContext {
        CacheReadContext { slot: 100, epoch: 3, maximum_slot_age: 5 }
    }
    #[test]
    fn offline_execution_rejects_simulation_and_confirmation() {
        use crate::{swqos::TradeType, trading::core::params::DexParamEnum};
        let (c, h, _) = fixture();
        let mut p = crate::instruction::stonkfun::tests::swap_params(
            TradeType::Buy,
            h.input_mint,
            h.output_mint,
            DexParamEnum::StonkFunSwap(c.cpmm(h, ctx()).unwrap()),
        );
        assert!(p.clone().without_rpc().is_err());
        p.simulate = false;
        p.wait_tx_confirmed = true;
        assert!(p.clone().without_rpc().is_err());
        p.wait_tx_confirmed = false;
        assert!(p.without_rpc().unwrap().rpc.is_none());
    }
    fn put(c: &mut SubscriptionAccountCache, k: Pubkey, owner: Pubkey, data: Vec<u8>) {
        c.update(k, CachedAccount { owner, data, slot: 100, write_version: 1 }).unwrap();
    }
    fn key(data: &mut [u8], offset: usize, k: Pubkey) {
        data[offset..offset + 32].copy_from_slice(k.as_ref());
    }
    fn num(data: &mut [u8], offset: usize, n: u64) {
        data[offset..offset + 8].copy_from_slice(&n.to_le_bytes());
    }
    pub(super) fn mint() -> Vec<u8> {
        let mut d = vec![0; 82];
        d[45] = 1;
        d
    }
    pub(crate) fn fixture() -> (SubscriptionAccountCache, PoolTradeHint, Pubkey) {
        let mut c = SubscriptionAccountCache::default();
        let pool = Pubkey::new_unique();
        let config = Pubkey::new_unique();
        let a = Pubkey::new_unique();
        let b = Pubkey::new_unique();
        let va = Pubkey::new_unique();
        let vb = Pubkey::new_unique();
        let program = raydium_cpmm::accounts::RAYDIUM_CPMM;
        let token = crate::constants::TOKEN_PROGRAM;
        let mut p = vec![0; 8 + raydium_cpmm_types::POOL_STATE_SIZE];
        p[..8].copy_from_slice(&raydium_cpmm_types::POOL_STATE_DISCRIMINATOR);
        for (offset, k) in [
            (8, config),
            (72, va),
            (104, vb),
            (168, a),
            (200, b),
            (232, token),
            (264, token),
            (296, Pubkey::new_unique()),
        ] {
            key(&mut p, offset, k);
        }
        num(&mut p, 341, 10);
        num(&mut p, 357, 20);
        num(&mut p, 397, 5);
        p[389] = 2;
        p[390] = 1;
        put(&mut c, pool, program, p);
        let mut f = vec![0; 8 + raydium_cpmm_types::AMM_CONFIG_SIZE];
        f[..8].copy_from_slice(&raydium_cpmm_types::AMM_CONFIG_DISCRIMINATOR);
        num(&mut f, 12, 4321);
        num(&mut f, 20, 12345);
        num(&mut f, 28, 54321);
        num(&mut f, 108, 987);
        put(&mut c, config, program, f);
        for (v, m, amount) in [(va, a, 1000), (vb, b, 2000)] {
            let mut d = vec![0; 165];
            key(&mut d, 0, m);
            key(&mut d, 32, raydium_cpmm::accounts::AUTHORITY);
            num(&mut d, 64, amount);
            d[108] = 1;
            put(&mut c, v, token, d);
            put(&mut c, m, token, mint());
        }
        (c, PoolTradeHint { pool, input_mint: b, output_mint: a }, va)
    }
    #[test]
    fn launchlab_uses_stock_quote_and_configured_fees() {
        let mut c = SubscriptionAccountCache::default();
        let pool = Pubkey::new_unique();
        let global = Pubkey::new_unique();
        let platform = Pubkey::new_unique();
        let base = Pubkey::new_unique();
        let quote = Pubkey::new_unique();
        let program = bonk::accounts::BONK;
        let token = crate::constants::TOKEN_PROGRAM;
        let mut p = vec![0; 8 + bonk_types::POOL_STATE_SIZE];
        p[..8].copy_from_slice(&bonk_types::POOL_STATE_DISCRIMINATOR);
        num(&mut p, 29, 777);
        num(&mut p, 37, 10000);
        num(&mut p, 45, 20000);
        for (offset, k) in [
            (141, global),
            (173, platform),
            (205, base),
            (237, quote),
            (269, Pubkey::new_unique()),
            (301, Pubkey::new_unique()),
            (333, Pubkey::new_unique()),
        ] {
            key(&mut p, offset, k);
        }
        put(&mut c, pool, program, p);
        let mut g = vec![0; 35];
        g[..8].copy_from_slice(&[149, 8, 156, 202, 160, 252, 176, 217]);
        num(&mut g, 27, 1234);
        let mut f = vec![0; 728];
        f[..8].copy_from_slice(&[160, 78, 128, 0, 248, 83, 230, 160]);
        num(&mut f, 104, 5678);
        num(&mut f, 720, 901);
        put(&mut c, global, program, g);
        put(&mut c, platform, program, f);
        put(&mut c, base, token, mint());
        put(&mut c, quote, token, mint());
        let h = PoolTradeHint { pool, input_mint: quote, output_mint: base };
        let p = c.launchlab(h, ctx()).unwrap();
        assert_eq!(p.quote_mint, quote);
        assert_eq!(p.total_base_sell, 777);
        assert_eq!((p.trade_fee_rate, p.platform_fee_rate, p.creator_fee_rate), (1234, 5678, 901));
        c.accounts.remove(&platform);
        assert!(c.launchlab(h, ctx()).is_err());
    }
    #[test]
    fn classic_mint_rejects_extended_layout_and_invalid_base_state() {
        let key = Pubkey::new_unique();
        let classic = crate::constants::TOKEN_PROGRAM;
        let token_2022 = Pubkey::new_from_array(spl_token_2022_interface::ID.to_bytes());
        // A valid Token-2022 MintCloseAuthority extension cannot be interpreted
        // as a classic mint merely by replacing the subscribed owner.
        let mut extended = mint();
        extended.resize(202, 0);
        extended[165] = 1;
        extended[166..168].copy_from_slice(&3u16.to_le_bytes());
        extended[168..170].copy_from_slice(&32u16.to_le_bytes());
        let mut cache = SubscriptionAccountCache::default();
        put(&mut cache, key, token_2022, extended.clone());
        assert!(cache.mint(key, Some(token_2022), ctx()).is_ok());
        for mut data in [mint(), extended, vec![0; 81], vec![0; 83]] {
            if data.len() == 82 {
                data[45] = 0;
            }
            let mut cache = SubscriptionAccountCache::default();
            put(&mut cache, key, classic, data);
            assert!(cache.mint(key, Some(classic), ctx()).is_err());
        }
        let mut cache = SubscriptionAccountCache::default();
        put(&mut cache, key, classic, mint());
        assert_eq!(cache.mint(key, Some(classic), ctx()).unwrap().1.basis_points, 0);
    }
    #[test]
    fn mint_fee_schedule_changes_with_supplied_epoch() {
        let mut c = SubscriptionAccountCache::default();
        let k = Pubkey::new_unique();
        let token = Pubkey::new_from_array(spl_token_2022_interface::ID.to_bytes());
        let mut d = vec![0; 278];
        d[45] = 1;
        d[165] = 1;
        d[166..168].copy_from_slice(&1u16.to_le_bytes());
        d[168..170].copy_from_slice(&108u16.to_le_bytes());
        num(&mut d, 242, 0);
        num(&mut d, 250, 500);
        d[258..260].copy_from_slice(&100u16.to_le_bytes());
        num(&mut d, 260, 4);
        num(&mut d, 268, 900);
        d[276..278].copy_from_slice(&300u16.to_le_bytes());
        put(&mut c, k, token, d);
        assert_eq!(c.mint(k, Some(token), ctx()).unwrap().1.basis_points, 100);
        let mut next = ctx();
        next.epoch = 4;
        assert_eq!(c.mint(k, Some(token), next).unwrap().1.basis_points, 300);
        // Validate the active schedule, without rejecting a distinct inactive one.
        c.accounts.get_mut(&k).unwrap().data[258..260].copy_from_slice(&10_001u16.to_le_bytes());
        assert!(c.mint(k, Some(token), ctx()).is_err());
        assert!(super::super::raydium_cpmm::token_transfer_fee_for_epoch(
            &c.accounts[&k].data,
            token,
            ctx().epoch,
        )
        .is_err());
        assert_eq!(c.mint(k, Some(token), next).unwrap().1.basis_points, 300);
        c.accounts.get_mut(&k).unwrap().data[258..260].copy_from_slice(&10_000u16.to_le_bytes());
        assert_eq!(c.mint(k, Some(token), ctx()).unwrap().1.basis_points, 10_000);
        let bytes = &c.accounts[&k].data;
        assert!(super::super::raydium_cpmm::token_transfer_fee_for_epoch(
            bytes,
            token,
            ctx().epoch
        )
        .is_ok());
    }
    #[test]
    fn stock_mint_extensions_require_public_transfers_and_usable_new_accounts() {
        use spl_token_2022_interface::extension::ExtensionType;
        let token = Pubkey::new_from_array(spl_token_2022_interface::ID.to_bytes());
        let key = Pubkey::new_unique();
        let build = |default_state: u8, paused: bool, active_hook: bool| {
            let mut data = vec![0; 166];
            data[45] = 1;
            data[165] = 1;
            let mut pause = vec![0; 33];
            pause[32] = u8::from(paused);
            let mut hook = vec![0; 64];
            hook[32] = u8::from(active_hook);
            for (kind, payload) in [
                (ExtensionType::DefaultAccountState, vec![default_state]),
                (ExtensionType::Pausable, pause),
                (ExtensionType::TransferHook, hook),
                (ExtensionType::ConfidentialTransferMint, vec![0; 65]),
            ] {
                data.extend_from_slice(&(kind as u16).to_le_bytes());
                data.extend_from_slice(&(payload.len() as u16).to_le_bytes());
                data.extend_from_slice(&payload);
            }
            data
        };
        let mut cache = SubscriptionAccountCache::default();
        put(&mut cache, key, token, build(1, false, false));
        assert!(cache.mint(key, Some(token), ctx()).is_ok());
        for (state, paused, hook, error) in [
            (2, false, false, "frozen/uninitialized"),
            (0, false, false, "frozen/uninitialized"),
            (1, true, false, "paused"),
            (1, false, true, "Active transfer hook"),
        ] {
            let mut cache = SubscriptionAccountCache::default();
            put(&mut cache, key, token, build(state, paused, hook));
            assert!(cache.mint(key, Some(token), ctx()).unwrap_err().to_string().contains(error));
        }
    }
    #[test]
    fn unsupported_mint_extensions_do_not_silently_produce_swap_params() {
        let mut c = SubscriptionAccountCache::default();
        let k = Pubkey::new_unique();
        let token = Pubkey::new_from_array(spl_token_2022_interface::ID.to_bytes());
        let mut data = vec![0; 170];
        data[45] = 1;
        data[165] = 1;
        // NonTransferable mint extension, zero-sized payload.
        data[166..168].copy_from_slice(&9u16.to_le_bytes());
        put(&mut c, k, token, data);
        let error = c.mint(k, Some(token), ctx()).unwrap_err();
        assert!(error.to_string().contains("Unsupported mint extension"));
    }
    #[test]
    fn cpmm_opening_time_uses_subscribed_clock() {
        let (mut cache, hint, _) = fixture();
        num(&mut cache.accounts.get_mut(&hint.pool).unwrap().data, 373, 1001);
        assert!(cache.cpmm_at(hint, ctx(), 1000).is_err());
        let checked = cache.cpmm_at(hint, ctx(), 1001).unwrap();
        let unchecked = cache.cpmm(hint, ctx()).unwrap();
        assert_eq!(checked.base_reserve, unchecked.base_reserve);
        assert_eq!(checked.quote_reserve, unchecked.quote_reserve);
        assert_eq!(checked.trade_fee_rate, unchecked.trade_fee_rate);
        // A not-yet-open pool is rejected before unrelated dependent-account work.
        cache.accounts.retain(|key, _| *key == hint.pool);
        assert!(cache
            .cpmm_at(hint, ctx(), 1000)
            .err()
            .expect("unopened pool must fail")
            .to_string()
            .contains("has not opened"));
        assert!(cache.cpmm_at(hint, ctx(), 1001).is_err());
    }
    #[test]
    #[ignore = "explicit local CPU benchmark; no network or transaction submission"]
    fn cpmm_at_single_decode_latency() {
        use std::{hint::black_box, time::Instant};
        let (cache, hint, _) = fixture();
        let read = ctx();
        let timestamp = u64::MAX;
        let legacy = || {
            let params = cache.cpmm(black_box(hint), black_box(read)).unwrap();
            let data = cache.get(hint.pool, raydium_cpmm::accounts::RAYDIUM_CPMM, read).unwrap();
            let pool = raydium_cpmm_types::pool_state_decode(black_box(&data[8..])).unwrap();
            assert!(black_box(timestamp) >= black_box(pool.open_time));
            black_box(params);
        };
        let current = || {
            black_box(
                cache.cpmm_at(black_box(hint), black_box(read), black_box(timestamp)).unwrap(),
            );
        };
        for _ in 0..1000 {
            legacy();
            current();
        }
        let mut legacy_samples = Vec::with_capacity(1000);
        let mut current_samples = Vec::with_capacity(1000);
        for batch in 0..1000 {
            let sample = |f: &dyn Fn(), output: &mut Vec<f64>| {
                let start = Instant::now();
                for _ in 0..100 {
                    f();
                }
                output.push(start.elapsed().as_nanos() as f64 / 100.0);
            };
            // Alternate order to reduce systematic warmup/thermal bias.
            if batch % 2 == 0 {
                sample(&legacy, &mut legacy_samples);
                sample(&current, &mut current_samples);
            } else {
                sample(&current, &mut current_samples);
                sample(&legacy, &mut legacy_samples);
            }
        }
        for (name, samples) in [
            ("legacy_double_decode", &mut legacy_samples),
            ("current_single_decode", &mut current_samples),
        ] {
            samples.sort_by(f64::total_cmp);
            println!("{name}: P50={:.2} ns P99={:.2} ns", samples[500], samples[990]);
        }
    }

    #[test]
    fn cpmm_vault_requires_valid_layout_authority_and_initialized_state() {
        let (cache, hint, vault) = fixture();
        assert!(cache.cpmm(hint, ctx()).is_ok());
        for case in 0..6 {
            let mut trial = cache.clone();
            let data = &mut trial.accounts.get_mut(&vault).unwrap().data;
            match case {
                0 => data[32..64].fill(0),
                1 => data[108] = 2, // frozen
                2 => data[108] = 0, // uninitialized
                3 => data[72..76].copy_from_slice(&2u32.to_le_bytes()), // invalid delegate COption
                4 => {
                    data.pop();
                }
                _ => data.push(0),
            }
            assert!(trial.cpmm(hint, ctx()).is_err(), "case {case}");
        }
        // Both base Token-2022 vaults and a valid account extension remain usable.
        let token = Pubkey::new_from_array(spl_token_2022_interface::ID.to_bytes());
        let mut extended = cache.clone();
        let pool = &mut extended.accounts.get_mut(&hint.pool).unwrap().data;
        key(pool, 232, token);
        extended.accounts.get_mut(&hint.output_mint).unwrap().owner = token;
        let account = extended.accounts.get_mut(&vault).unwrap();
        account.owner = token;
        assert!(extended.cpmm(hint, ctx()).is_ok());
        let data = &mut extended.accounts.get_mut(&vault).unwrap().data;
        data.resize(170, 0);
        data[165] = 2; // Account
        data[166..168].copy_from_slice(&7u16.to_le_bytes()); // ImmutableOwner
        assert!(extended.cpmm(hint, ctx()).is_ok());
        extended.accounts.get_mut(&vault).unwrap().data[165] = 1; // Mint layout
        assert!(extended.cpmm(hint, ctx()).is_err());
    }
    #[test]
    fn reverse_trade_keeps_pool_orientation_and_actual_fees() {
        let (c, h, _) = fixture();
        let p = c.cpmm(h, ctx()).unwrap();
        assert_eq!(p.base_mint, h.output_mint);
        assert_eq!(p.quote_mint, h.input_mint);
        assert_eq!((p.base_reserve, p.quote_reserve), (965, 2000));
        assert_eq!((p.trade_fee_rate, p.creator_fee_rate, p.creator_fee_on), (4321, 987, 2));
    }
    #[test]
    fn rejects_missing_stale_owner_pair_and_fee_underflow() {
        let (mut c, h, v) = fixture();
        let mut stale = ctx();
        stale.slot = 106;
        assert!(c.cpmm(h, stale).is_err());
        let mut wrong = h;
        wrong.input_mint = Pubkey::new_unique();
        assert!(c.cpmm(wrong, ctx()).is_err());
        c.accounts.get_mut(&v).unwrap().owner = Pubkey::new_unique();
        assert!(c.cpmm(h, ctx()).is_err());
        c.accounts.get_mut(&v).unwrap().owner = crate::constants::TOKEN_PROGRAM;
        num(&mut c.accounts.get_mut(&v).unwrap().data, 64, 1);
        assert!(c.cpmm(h, ctx()).is_err());
        c.accounts.remove(&v);
        assert!(c.cpmm(h, ctx()).is_err());
    }
    #[cfg(feature = "parser-adapter")]
    #[test]
    fn parser_raw_updates_preserve_versions_and_account_closures() {
        use sol_parser_sdk::accounts::{liquidity_snapshot::RawAccountSnapshotEvent, AccountData};
        let (mut cache, _, vault) = fixture();
        let old = cache.accounts[&vault].clone();
        let mut event = RawAccountSnapshotEvent {
            metadata: sol_parser_sdk::core::events::EventMetadata {
                slot: 100,
                ..Default::default()
            },
            account: AccountData {
                pubkey: vault,
                owner: old.owner,
                data: old.data,
                lamports: 1,
                executable: false,
                rent_epoch: 0,
            },
            write_version: 1,
            is_startup: false,
        };
        assert!(!cache.update_from_parser_snapshot(&event).unwrap());
        event.write_version = 2;
        event.account.lamports = 0;
        assert!(cache.update_from_parser_snapshot(&event).unwrap());
        assert!(cache.accounts[&vault].data.is_empty());
        event.write_version = 1;
        event.account.lamports = 1;
        assert!(!cache.update_from_parser_snapshot(&event).unwrap());
        assert!(cache.accounts[&vault].data.is_empty());
    }
    #[test]
    fn update_order_and_conflicts_are_explicit() {
        let (mut c, _, v) = fixture();
        let mut a = c.accounts[&v].clone();
        assert!(!c.update(v, a.clone()).unwrap());
        a.slot = 99;
        assert!(!c.update(v, a.clone()).unwrap());
        a.slot = 100;
        a.data[0] ^= 1;
        assert!(c.update(v, a.clone()).is_err());
        a.write_version = 2;
        assert!(c.update(v, a).unwrap());
    }
}
