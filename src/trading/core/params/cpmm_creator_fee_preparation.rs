//! Prepare creator-fee collection entirely from subscription-fed account bytes.
use super::{CacheReadContext, SubscriptionAccountCache};
use crate::instruction::{
    raydium_cpmm::build_creator_fee_collection,
    utils::{
        raydium_cpmm::{
            accounts, estimate_creator_fee_payout, get_creator_fee_share_pda,
            resolve_creator_fee_share_rate_from_data,
        },
        raydium_cpmm_types::{
            amm_config_decode, pool_state_decode, AMM_CONFIG_DISCRIMINATOR,
            POOL_STATE_DISCRIMINATOR,
        },
    },
};
use anyhow::{anyhow, ensure, Result};
use solana_sdk::{instruction::Instruction, pubkey::Pubkey};

/// Reprepare after account updates. Payouts exclude Token-2022 transfer fees and
/// the on-chain rate can change before execution. No RPC/signer/submission is used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedCpmmCreatorFeeCollection {
    pub instruction: Instruction,
    /// Read context used for the estimate; no chain execution guarantee.
    pub snapshot_slot: u64,
    /// Pool, AmmConfig, CreatorFeeShare: (address, slot, write_version).
    pub account_versions: [(Pubkey, u64, u64); 3],
    pub pool: Pubkey,
    pub creator: Pubkey,
    pub amm_config: Pubkey,
    pub creator_fee_share: Pubkey,
    pub share_rate: u64,
    pub creator_payout_token0: u64,
    pub creator_payout_token1: u64,
    pub protocol_share_token0: u64,
    pub protocol_share_token1: u64,
}

impl SubscriptionAccountCache {
    /// `payer=None` uses creator-signed collection; `Some` uses permissionless.
    /// Requires pool, config and an explicit share-PDA observation. Missing cache
    /// entries mean unknown, not absent on-chain. A validated bootstrap absence or
    /// account-closure update may be stored as an empty CachedAccount at the PDA.
    /// Apply complete gRPC batches before exposing a stable cache to readers.
    pub fn prepare_cpmm_creator_fee_collection(
        &self,
        pool_address: Pubkey,
        payer: Option<Pubkey>,
        ctx: CacheReadContext,
    ) -> Result<PreparedCpmmCreatorFeeCollection> {
        let program = accounts::RAYDIUM_CPMM;
        let data = self.get(pool_address, program, ctx)?;
        ensure!(
            data.get(..8) == Some(POOL_STATE_DISCRIMINATOR.as_slice()),
            "Invalid cached CPMM pool discriminator"
        );
        let pool =
            pool_state_decode(&data[8..]).ok_or_else(|| anyhow!("Invalid cached CPMM pool"))?;
        let token_2022 = Pubkey::new_from_array(spl_token_2022_interface::ID.to_bytes());
        ensure!(
            [pool.token0_program, pool.token1_program].iter().all(|program| {
                *program == crate::constants::TOKEN_PROGRAM || *program == token_2022
            }),
            "Unsupported CPMM token program in creator-fee collection"
        );
        ensure!(
            pool.token0_mint != Pubkey::default()
                && pool.token1_mint != Pubkey::default()
                && pool.token0_mint != pool.token1_mint,
            "Invalid CPMM mint pair in creator-fee collection"
        );
        ensure!(
            pool.creator_fees_token0 != 0 || pool.creator_fees_token1 != 0,
            "No accrued CPMM creator fees to collect"
        );
        let data = self.get(pool.amm_config, program, ctx)?;
        ensure!(
            data.get(..8) == Some(AMM_CONFIG_DISCRIMINATOR.as_slice()),
            "Invalid cached CPMM config discriminator"
        );
        let config =
            amm_config_decode(&data[8..]).ok_or_else(|| anyhow!("Invalid cached CPMM config"))?;
        let share_address = get_creator_fee_share_pda(&pool.pool_creator, &pool.amm_config);
        let share = self.accounts.get(&share_address)
            .ok_or_else(|| anyhow!("Unknown cached CreatorFeeShare PDA: {share_address}; a stream with no update does not prove absence"))?;
        // Validate age even for absent/foreign-owned observations.
        let data = self.get(share_address, share.owner, ctx)?;
        let rate = resolve_creator_fee_share_rate_from_data(
            &config,
            &pool.pool_creator,
            &pool.amm_config,
            Some((share.owner, data)),
        )?;
        let (payout0, payout1) = estimate_creator_fee_payout(&pool, rate)?;
        let protocol0 = pool.creator_fees_token0 - payout0;
        let protocol1 = pool.creator_fees_token1 - payout1;
        // Collection books these shares into existing protocol counters. Reject
        // a locally impossible settlement before allocating instruction metas.
        ensure!(
            pool.protocol_fees_token0.checked_add(protocol0).is_some()
                && pool.protocol_fees_token1.checked_add(protocol1).is_some(),
            "CPMM protocol fee overflow at creator-fee collection"
        );
        let instruction = build_creator_fee_collection(&pool_address, &pool, payer, share_address);
        let version = |key: Pubkey| -> Result<(Pubkey, u64, u64)> {
            let account =
                self.accounts.get(&key).ok_or_else(|| anyhow!("Missing cached account: {key}"))?;
            Ok((key, account.slot, account.write_version))
        };
        Ok(PreparedCpmmCreatorFeeCollection {
            instruction,
            snapshot_slot: ctx.slot,
            account_versions: [
                version(pool_address)?,
                version(pool.amm_config)?,
                version(share_address)?,
            ],
            pool: pool_address,
            creator: pool.pool_creator,
            amm_config: pool.amm_config,
            creator_fee_share: share_address,
            share_rate: rate,
            creator_payout_token0: payout0,
            creator_payout_token1: payout1,
            protocol_share_token0: protocol0,
            protocol_share_token1: protocol1,
        })
    }
    /// Check immediately before using a prepared estimate/instruction. Changed
    /// versions, stale state, cleared fees and modified preparation fields fail.
    /// This validates local cache state only; still check subscription continuity
    /// and caller fork policy. The protocol reads the rate when executing.
    pub fn validate_cpmm_creator_fee_collection(
        &self,
        prepared: &PreparedCpmmCreatorFeeCollection,
        payer: Option<Pubkey>,
        ctx: CacheReadContext,
    ) -> Result<()> {
        ensure!(ctx.slot >= prepared.snapshot_slot, "Collection read context moved backwards");
        ensure!(
            prepared.account_versions.iter().all(|(_, slot, _)| *slot <= prepared.snapshot_slot),
            "Prepared snapshot precedes its account observations"
        );
        // Reject invalidated preparations before decoding accounts, deriving PDAs
        // or allocating another instruction. Public fields still receive the
        // complete reconstruction comparison below when versions match.
        for (key, slot, write_version) in &prepared.account_versions {
            ensure!(
                self.accounts.get(key).is_some_and(|account| {
                    (account.slot, account.write_version) == (*slot, *write_version)
                }),
                "Creator-fee preparation changed; prepare again"
            );
        }
        let mut current = self.prepare_cpmm_creator_fee_collection(prepared.pool, payer, ctx)?;
        // Advancing the subscribed block slot alone must not invalidate otherwise
        // identical, still-fresh account observations.
        current.snapshot_slot = prepared.snapshot_slot;
        ensure!(current == *prepared, "Creator-fee preparation changed; prepare again");
        Ok(())
    }
}
