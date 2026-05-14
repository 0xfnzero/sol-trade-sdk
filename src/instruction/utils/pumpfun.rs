//! Pump.fun bonding-curve utilities (flat module): PDAs, fee `#2`, pump-fees `SharingConfig`, cold RPC.
//!
//! Hot swap ix assembly stays sync; async helpers at file bottom. Layout matches `@pump-fun/pump-sdk`.

use crate::common::{bonding_curve::BondingCurveAccount, SolanaRpcClient};
use anyhow::anyhow;
use borsh::BorshDeserialize;
use rand::seq::IndexedRandom;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use std::sync::Arc;

// --- seeds -------------------------------------------------------------

pub mod seeds {

    /// Seed for bonding curve PDAs (`["bonding-curve", mint]`).
    pub const BONDING_CURVE_SEED: &[u8] = b"bonding-curve";
    /// Seed for bonding curve v2 PDA (`["bonding-curve-v2", mint]`).
    pub const BONDING_CURVE_V2_SEED: &[u8] = b"bonding-curve-v2";
    /// Creator vault PDA seeds prefix (`["creator-vault", authority]`).
    pub const CREATOR_VAULT_SEED: &[u8] = b"creator-vault";
    /// Metadata PDA seeds prefix.
    pub const METADATA_SEED: &[u8] = b"metadata";
    /// User volume accumulator for cashback / bonding-curve UX.
    pub const USER_VOLUME_ACCUMULATOR_SEED: &[u8] = b"user_volume_accumulator";
    /// Global volume accumulator.
    pub const GLOBAL_VOLUME_ACCUMULATOR_SEED: &[u8] = b"global_volume_accumulator";
    pub const FEE_CONFIG_SEED: &[u8] = b"fee_config";
    /// `feeSharingConfig` PDA under pump-fees (`feeSharingConfigPda`).
    pub const SHARING_CONFIG_SEED: &[u8] = b"sharing-config";
}

pub mod global_constants {

    use solana_sdk::{pubkey, pubkey::Pubkey};

    pub const INITIAL_VIRTUAL_TOKEN_RESERVES: u64 = 1_073_000_000_000_000;
    pub const INITIAL_VIRTUAL_SOL_RESERVES: u64 = 30_000_000_000;
    pub const INITIAL_REAL_TOKEN_RESERVES: u64 = 793_100_000_000_000;
    pub const TOKEN_TOTAL_SUPPLY: u64 = 1_000_000_000_000_000;
    pub const FEE_BASIS_POINTS: u64 = 95;
    pub const ENABLE_MIGRATE: bool = false;
    pub const POOL_MIGRATION_FEE: u64 = 15_000_001;
    pub const CREATOR_FEE: u64 = 30;
    pub const SCALE: u64 = 1_000_000;
    pub const LAMPORTS_PER_SOL: u64 = 1_000_000_000;
    pub const COMPLETION_LAMPORTS: u64 = 85 * LAMPORTS_PER_SOL;

    pub const FEE_RECIPIENT: Pubkey = pubkey!("62qc2CNXwrYqQScmEdiZFFAnJR262PxWEuNQtxfafNgV");
    pub const FEE_RECIPIENT_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: FEE_RECIPIENT,
            is_signer: false,
            is_writable: true,
        };

    pub const MAYHEM_FEE_RECIPIENTS: [Pubkey; 8] = [
        pubkey!("GesfTA3X2arioaHp8bbKdjG9vJtskViWACZoYvxp4twS"),
        pubkey!("4budycTjhs9fD6xw62VBducVTNgMgJJ5BgtKq7mAZwn6"),
        pubkey!("8SBKzEQU4nLSzcwF4a74F2iaUDQyTfjGndn6qUWBnrpR"),
        pubkey!("4UQeTP1T39KZ9Sfxzo3WR5skgsaP6NZa87BAkuazLEKH"),
        pubkey!("8sNeir4QsLsJdYpc9RZacohhK1Y5FLU3nC5LXgYB4aa6"),
        pubkey!("Fh9HmeLNUMVCvejxCtCL2DbYaRyBFVJ5xrWkLnMH6fdk"),
        pubkey!("463MEnMeGyJekNZFQSTUABBEbLnvMTALbT6ZmsxAbAdq"),
        pubkey!("6AUH3WEHucYZyC61hqpqYUWVto5qA5hjHuNQ32GNnNxA"),
    ];
    pub const MAYHEM_FEE_RECIPIENT: Pubkey = MAYHEM_FEE_RECIPIENTS[0];
    pub const MAYHEM_FEE_RECIPIENT_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: MAYHEM_FEE_RECIPIENT,
            is_signer: false,
            is_writable: true,
        };

    pub const GLOBAL_ACCOUNT: Pubkey = pubkey!("4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf");
    pub const GLOBAL_ACCOUNT_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: GLOBAL_ACCOUNT,
            is_signer: false,
            is_writable: false,
        };

    pub const AUTHORITY: Pubkey = pubkey!("FFWtrEQ4B4PKQoVuHYzZq8FabGkVatYzDpEVHsK5rrhF");
    pub const WITHDRAW_AUTHORITY: Pubkey = pubkey!("39azUYFWPz3VHgKCf3VChUwbpURdCHRxjWVowf5jUJjg");

    pub const PUMPFUN_AMM_FEE_1: Pubkey = pubkey!("7VtfL8fvgNfhz17qKRMjzQEXgbdpnHHHQRh54R9jP2RJ");
    pub const PUMPFUN_AMM_FEE_2: Pubkey = pubkey!("7hTckgnGnLQR6sdH7YkqFTAA7VwTfYFaZ6EhEsU3saCX");
    pub const PUMPFUN_AMM_FEE_3: Pubkey = pubkey!("9rPYyANsfQZw3DnDmKE3YCQF5E8oD89UXoHn9JFEhJUz");
    pub const PUMPFUN_AMM_FEE_4: Pubkey = pubkey!("AVmoTthdrX6tKt4nDjco2D775W2YK3sDhxPcMmzUAmTY");
    pub const PUMPFUN_AMM_FEE_5: Pubkey = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    pub const PUMPFUN_AMM_FEE_6: Pubkey = pubkey!("FWsW1xNtWscwNmKv6wVsU1iTzRN6wmmk3MjxRP5tT7hz");
    pub const PUMPFUN_AMM_FEE_7: Pubkey = pubkey!("G5UZAVbAf46s7cKWoyKu8kYTip9DGTpbLZ2qa9Aq69dP");

    pub const PROTOCOL_EXTRA_FEE_RECIPIENTS: [Pubkey; 8] = [
        pubkey!("5YxQFdt3Tr9zJLvkFccqXVUwhdTWJQc1fFg2YPbxvxeD"),
        pubkey!("9M4giFFMxmFGXtc3feFzRai56WbBqehoSeRE5GK7gf7"),
        pubkey!("GXPFM2caqTtQYC2cJ5yJRi9VDkpsYZXzYdwYpGnLmtDL"),
        pubkey!("3BpXnfJaUTiwXnJNe7Ej1rcbzqTTQUvLShZaWazebsVR"),
        pubkey!("5cjcW9wExnJJiqgLjq7DEG75Pm6JBgE1hNv4B2vHXUW6"),
        pubkey!("EHAAiTxcdDwQ3U4bU6YcMsQGaekdzLS3B5SmYo46kJtL"),
        pubkey!("5eHhjP8JaYkz83CWwvGU2uMUXefd3AazWGx4gpcuEEYD"),
        pubkey!("A7hAgCzFw14fejgCp387JUJRMNyz4j89JKnhtKU8piqW"),
    ];

    /// Buyback fee recipients (v2 account #9 in buy_v2/sell_v2).
    /// 对应官方 FEE_RECIPIENTS.md "Buyback (Applies to All)" 池，与主 fee_recipient 池互斥。
    pub const BUYBACK_FEE_RECIPIENTS: [Pubkey; 8] = [
        pubkey!("5YxQFdt3Tr9zJLvkFccqXVUwhdTWJQc1fFg2YPbxvxeD"),
        pubkey!("9M4giFFMxmFGXtc3feFzRai56WbBqehoSeRE5GK7gf7"),
        pubkey!("GXPFM2caqTtQYC2cJ5yJRi9VDkpsYZXzYdwYpGnLmtDL"),
        pubkey!("3BpXnfJaUTiwXnJNe7Ej1rcbzqTTQUvLShZaWazebsVR"),
        pubkey!("5cjcW9wExnJJiqgLjq7DEG75Pm6JBgE1hNv4B2vHXUW6"),
        pubkey!("EHAAiTxcdDwQ3U4bU6YcMsQGaekdzLS3B5SmYo46kJtL"),
        pubkey!("5eHhjP8JaYkz83CWwvGU2uMUXefd3AazWGx4gpcuEEYD"),
        pubkey!("A7hAgCzFw14fejgCp387JUJRMNyz4j89JKnhtKU8piqW"),
    ];
}

pub mod accounts {

    use solana_sdk::{pubkey, pubkey::Pubkey};

    pub const PUMPFUN: Pubkey = pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P");
    pub const MPL_TOKEN_METADATA: Pubkey = pubkey!("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s");
    pub const EVENT_AUTHORITY: Pubkey = pubkey!("Ce6TQqeHC9p8KetsN6JsjHK7UTZk7nasjjnr7XxXp9F1");
    pub const ASSOCIATED_TOKEN_PROGRAM: Pubkey =
        pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
    pub const AMM_PROGRAM: Pubkey = pubkey!("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8");
    pub const FEE_PROGRAM: Pubkey = pubkey!("pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ");
    pub const GLOBAL_VOLUME_ACCUMULATOR: Pubkey =
        pubkey!("Hq2wp8uJ9jCPsYgNHex8RtqdvMPfVGoYwjvF1ATiwn2Y");
    pub const FEE_CONFIG: Pubkey = pubkey!("8Wf5TiAheLUqBrKXeYg2JtAFFMWtKdG2BSFgqUcPVwTt");

    pub const PUMPFUN_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: PUMPFUN,
            is_signer: false,
            is_writable: false,
        };

    pub const EVENT_AUTHORITY_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: EVENT_AUTHORITY,
            is_signer: false,
            is_writable: false,
        };

    pub const FEE_PROGRAM_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: FEE_PROGRAM,
            is_signer: false,
            is_writable: false,
        };

    pub const GLOBAL_VOLUME_ACCUMULATOR_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: GLOBAL_VOLUME_ACCUMULATOR,
            is_signer: false,
            is_writable: true,
        };

    pub const FEE_CONFIG_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: FEE_CONFIG,
            is_signer: false,
            is_writable: false,
        };
}

// --- Anchor / layout constants ---------------------------------------

/// Minimum bonding curve account data length (`sdk.ts` `BONDING_CURVE_NEW_SIZE`).
pub const PUMP_BONDING_CURVE_MIN_DATA_LEN: usize = 151;

pub const BUY_DISCRIMINATOR: [u8; 8] = [102, 6, 61, 18, 1, 218, 235, 234];
pub const BUY_EXACT_SOL_IN_DISCRIMINATOR: [u8; 8] = [56, 252, 116, 8, 158, 223, 205, 95];
pub const SELL_DISCRIMINATOR: [u8; 8] = [51, 230, 133, 164, 1, 127, 131, 173];

/// `buy_v2` — unified SOL/USDC quote interface ([pump-public-docs](https://github.com/pump-fun/pump-public-docs)).
pub const BUY_V2_DISCRIMINATOR: [u8; 8] = [184, 23, 238, 97, 103, 197, 211, 61];
/// `sell_v2`
pub const SELL_V2_DISCRIMINATOR: [u8; 8] = [93, 246, 130, 60, 231, 233, 64, 178];
/// `buy_exact_quote_in_v2` (native SOL spend for SOL-paired coins when `quote_mint` is WSOL)
pub const BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR: [u8; 8] = [194, 171, 28, 70, 104, 77, 91, 47];

pub const EXTEND_ACCOUNT_DISCRIMINATOR: [u8; 8] = [234, 102, 194, 203, 150, 72, 62, 229];

pub const SHARING_CONFIG_ACCOUNT_DISCRIMINATOR: [u8; 8] = [216, 74, 9, 0, 56, 140, 93, 75];

pub(crate) const SHARING_CONFIG_STATUS_ACTIVE: u8 = 1;

// --- Fee recipient pools -----------------------------------------------

#[inline]
pub fn is_mayhem_fee_recipient(pubkey: &Pubkey) -> bool {
    global_constants::MAYHEM_FEE_RECIPIENTS.contains(pubkey)
}

#[inline]
pub fn is_amm_fee_recipient(pubkey: &Pubkey) -> bool {
    pubkey == &global_constants::PUMPFUN_AMM_FEE_1
        || pubkey == &global_constants::PUMPFUN_AMM_FEE_2
        || pubkey == &global_constants::PUMPFUN_AMM_FEE_3
        || pubkey == &global_constants::PUMPFUN_AMM_FEE_4
        || pubkey == &global_constants::PUMPFUN_AMM_FEE_5
        || pubkey == &global_constants::PUMPFUN_AMM_FEE_6
        || pubkey == &global_constants::PUMPFUN_AMM_FEE_7
}

#[inline]
pub fn is_standard_bonding_fee_recipient(pubkey: &Pubkey) -> bool {
    *pubkey == global_constants::FEE_RECIPIENT || is_amm_fee_recipient(pubkey)
}

#[inline]
pub fn reconcile_mayhem_mode_for_trade(
    mayhem_from_event: Option<bool>,
    fee_recipient: &Pubkey,
) -> bool {
    if *fee_recipient == Pubkey::default() {
        return mayhem_from_event.unwrap_or(false);
    }
    let fee_m = is_mayhem_fee_recipient(fee_recipient);
    let fee_s = is_standard_bonding_fee_recipient(fee_recipient);
    match mayhem_from_event {
        Some(log_m) => {
            if fee_m && !log_m {
                true
            } else if fee_s && log_m && !fee_m {
                false
            } else {
                log_m
            }
        }
        None => fee_m,
    }
}

#[inline]
pub fn fee_recipient_ok_for_bonding_curve_mode(pk: &Pubkey, is_mayhem_mode: bool) -> bool {
    let is_m = is_mayhem_fee_recipient(pk);
    let is_s = is_standard_bonding_fee_recipient(pk);
    if is_mayhem_mode {
        !(is_s && !is_m)
    } else {
        !(is_m && !is_s)
    }
}

#[inline]
pub fn get_mayhem_fee_recipient_meta_random() -> AccountMeta {
    let recipient = *global_constants::MAYHEM_FEE_RECIPIENTS
        .choose(&mut rand::rng())
        .unwrap_or(&global_constants::MAYHEM_FEE_RECIPIENTS[0]);
    AccountMeta { pubkey: recipient, is_signer: false, is_writable: true }
}

#[inline]
pub fn get_standard_fee_recipient_meta_random() -> AccountMeta {
    const POOL: &[Pubkey] = &[
        global_constants::FEE_RECIPIENT,
        global_constants::PUMPFUN_AMM_FEE_1,
        global_constants::PUMPFUN_AMM_FEE_2,
        global_constants::PUMPFUN_AMM_FEE_3,
        global_constants::PUMPFUN_AMM_FEE_4,
        global_constants::PUMPFUN_AMM_FEE_5,
        global_constants::PUMPFUN_AMM_FEE_6,
        global_constants::PUMPFUN_AMM_FEE_7,
    ];
    let recipient = *POOL.choose(&mut rand::rng()).unwrap_or(&global_constants::FEE_RECIPIENT);
    AccountMeta { pubkey: recipient, is_signer: false, is_writable: true }
}

#[inline]
pub fn get_protocol_extra_fee_recipient_random() -> Pubkey {
    *global_constants::PROTOCOL_EXTRA_FEE_RECIPIENTS
        .choose(&mut rand::rng())
        .unwrap_or(&global_constants::PROTOCOL_EXTRA_FEE_RECIPIENTS[0])
}

/// Buyback fee recipient (#9 in buy_v2/sell_v2) — dedicated pool, distinct from protocol extra fee recipients.
#[inline]
pub fn get_buyback_fee_recipient_random() -> Pubkey {
    *global_constants::BUYBACK_FEE_RECIPIENTS
        .choose(&mut rand::rng())
        .unwrap_or(&global_constants::BUYBACK_FEE_RECIPIENTS[0])
}

#[inline]
pub fn pump_fun_fee_recipient_meta(
    observed_fee_recipient: Pubkey,
    is_mayhem_mode: bool,
) -> AccountMeta {
    let trust_observation = observed_fee_recipient != Pubkey::default()
        && fee_recipient_ok_for_bonding_curve_mode(&observed_fee_recipient, is_mayhem_mode);
    if trust_observation {
        AccountMeta { pubkey: observed_fee_recipient, is_signer: false, is_writable: true }
    } else if is_mayhem_mode {
        get_mayhem_fee_recipient_meta_random()
    } else {
        get_standard_fee_recipient_meta_random()
    }
}

// --- Extend bonding curve (cold path) --------------------------------

#[inline]
pub fn extend_bonding_curve_account_instruction(
    bonding_curve: &Pubkey,
    user: &Pubkey,
) -> Instruction {
    Instruction::new_with_bytes(
        accounts::PUMPFUN,
        &EXTEND_ACCOUNT_DISCRIMINATOR,
        vec![
            AccountMeta::new(*bonding_curve, false),
            AccountMeta::new(*user, true),
            crate::constants::SYSTEM_PROGRAM_META,
            accounts::EVENT_AUTHORITY_META,
            accounts::PUMPFUN_META,
        ],
    )
}

// --- Cached PDAs + creator_vault resolve ------------------------------

#[inline]
pub fn get_bonding_curve_pda(mint: &Pubkey) -> Option<Pubkey> {
    crate::common::fast_fn::get_cached_pda(
        crate::common::fast_fn::PdaCacheKey::PumpFunBondingCurve(*mint),
        || {
            let seeds: &[&[u8]; 2] = &[seeds::BONDING_CURVE_SEED, mint.as_ref()];
            let program_id: &Pubkey = &accounts::PUMPFUN;
            Pubkey::try_find_program_address(seeds, program_id).map(|pubkey| pubkey.0)
        },
    )
}

#[inline]
pub fn get_bonding_curve_v2_pda(mint: &Pubkey) -> Option<Pubkey> {
    crate::common::fast_fn::get_cached_pda(
        crate::common::fast_fn::PdaCacheKey::PumpFunBondingCurveV2(*mint),
        || {
            let seeds: &[&[u8]; 2] = &[seeds::BONDING_CURVE_V2_SEED, mint.as_ref()];
            let program_id: &Pubkey = &accounts::PUMPFUN;
            Pubkey::try_find_program_address(seeds, program_id).map(|pubkey| pubkey.0)
        },
    )
}

#[inline]
pub fn get_creator(creator_vault_pda: &Pubkey) -> Pubkey {
    if creator_vault_pda.eq(&Pubkey::default()) {
        Pubkey::default()
    } else {
        static DEFAULT_CREATOR_VAULT: std::sync::LazyLock<Option<Pubkey>> =
            std::sync::LazyLock::new(|| get_creator_vault_pda(&Pubkey::default()));
        match DEFAULT_CREATOR_VAULT.as_ref() {
            Some(default) if creator_vault_pda.eq(default) => Pubkey::default(),
            _ => *creator_vault_pda,
        }
    }
}

#[inline]
pub fn get_creator_vault_pda(creator: &Pubkey) -> Option<Pubkey> {
    crate::common::fast_fn::get_cached_pda(
        crate::common::fast_fn::PdaCacheKey::PumpFunCreatorVault(*creator),
        || {
            let seeds: &[&[u8]; 2] = &[seeds::CREATOR_VAULT_SEED, creator.as_ref()];
            let program_id: &Pubkey = &accounts::PUMPFUN;
            Pubkey::try_find_program_address(seeds, program_id).map(|pubkey| pubkey.0)
        },
    )
}

#[inline]
pub fn get_fee_sharing_config_pda(mint: &Pubkey) -> Option<Pubkey> {
    crate::common::fast_fn::get_cached_pda(
        crate::common::fast_fn::PdaCacheKey::PumpFunFeeSharingConfig(*mint),
        || {
            Pubkey::try_find_program_address(
                &[seeds::SHARING_CONFIG_SEED, mint.as_ref()],
                &accounts::FEE_PROGRAM,
            )
            .map(|(p, _)| p)
        },
    )
}

#[inline]
pub fn phantom_default_creator_vault() -> Pubkey {
    solana_sdk::pubkey!("2DR3iqRPVThyRLVJnwjPW1qiGWrp8RUFfHVjMbZyhdNc")
}

#[inline]
pub fn is_phantom_default_creator_vault(pk: &Pubkey) -> bool {
    *pk == phantom_default_creator_vault()
}

#[inline]
pub fn resolve_creator_vault_for_ix(
    creator: &Pubkey,
    creator_vault_from_event: Pubkey,
    mint: &Pubkey,
) -> Option<Pubkey> {
    resolve_creator_vault_for_ix_with_fee_sharing(creator, creator_vault_from_event, mint, None)
}

/// Resolves Pump.fun bonding-curve buy/sell **account `#10` (`creator_vault`)** for ix assembly.
///
/// **Priority (highest first)**  
/// 1. **Explicit `creator_vault`** from ix / parser / cached observation when non-default and not the phantom
///    sentinel — **always used as-is** (no remap to [`get_creator_vault_pda`] from `creator`);
///    fee-sharing / multi-party layouts rely on upstream passing the vault the program expects (`pfee…` / `update_fee_shares`).
/// 2. If ix vault missing: optional `fee_sharing_creator_vault_if_active` hint (non-default, non-phantom).
/// 3. Else: [`get_creator_vault_pda`] from `creator` when `creator` is known.
#[inline]
pub fn resolve_creator_vault_for_ix_with_fee_sharing(
    creator: &Pubkey,
    creator_vault_from_event: Pubkey,
    _mint: &Pubkey,
    fee_sharing_creator_vault_if_active: Option<Pubkey>,
) -> Option<Pubkey> {
    let phantom = phantom_default_creator_vault();

    if creator_vault_from_event != Pubkey::default() && creator_vault_from_event != phantom {
        return Some(creator_vault_from_event);
    }

    if let Some(v) = fee_sharing_creator_vault_if_active {
        if v != Pubkey::default() && v != phantom {
            return Some(v);
        }
    }

    if *creator == Pubkey::default() {
        return None;
    }

    get_creator_vault_pda(creator)
}

#[inline]
pub fn get_user_volume_accumulator_pda(user: &Pubkey) -> Option<Pubkey> {
    crate::common::fast_fn::get_cached_pda(
        crate::common::fast_fn::PdaCacheKey::PumpFunUserVolume(*user),
        || {
            let seed: &[&[u8]; 2] = &[seeds::USER_VOLUME_ACCUMULATOR_SEED, user.as_ref()];
            let program_id: &Pubkey = &accounts::PUMPFUN;
            Pubkey::try_find_program_address(seed, program_id).map(|pubkey| pubkey.0)
        },
    )
}

#[inline]
pub fn get_buy_price(
    amount: u64,
    virtual_sol_reserves: u64,
    virtual_token_reserves: u64,
    real_token_reserves: u64,
) -> u64 {
    if amount == 0 {
        return 0;
    }

    let n: u128 = (virtual_sol_reserves as u128) * (virtual_token_reserves as u128);
    let i: u128 = (virtual_sol_reserves as u128) + (amount as u128);
    let r: u128 = n / i + 1;
    let s: u128 = (virtual_token_reserves as u128) - r;
    let s_u64 = s as u64;

    s_u64.min(real_token_reserves)
}

// --- RPC -------------------------------------------------------------

#[inline]
pub async fn fetch_fee_sharing_creator_vault_if_active(
    rpc: &SolanaRpcClient,
    mint: &Pubkey,
) -> Result<Option<Pubkey>, anyhow::Error> {
    let Some(config_pda) = get_fee_sharing_config_pda(mint) else {
        return Ok(None);
    };
    let acc = match rpc.get_account(&config_pda).await {
        Ok(a) => a,
        Err(_) => return Ok(None),
    };
    if acc.owner != accounts::FEE_PROGRAM {
        return Ok(None);
    }
    let d = acc.data.as_slice();
    if d.len() < 43 || d[..8] != SHARING_CONFIG_ACCOUNT_DISCRIMINATOR {
        return Ok(None);
    }
    if d[10] != SHARING_CONFIG_STATUS_ACTIVE {
        return Ok(None);
    }
    let mint_on_chain = Pubkey::new_from_array(
        d[11..43].try_into().map_err(|_| anyhow!("SharingConfig mint slice"))?,
    );
    if mint_on_chain != *mint {
        return Ok(None);
    }
    Ok(get_creator_vault_pda(&config_pda))
}

#[inline]
pub async fn fetch_bonding_curve_account(
    rpc: &SolanaRpcClient,
    mint: &Pubkey,
) -> Result<(Arc<BondingCurveAccount>, Pubkey), anyhow::Error> {
    let bonding_curve_pda: Pubkey =
        get_bonding_curve_pda(mint).ok_or_else(|| anyhow!("Bonding curve not found"))?;

    let account = rpc.get_account(&bonding_curve_pda).await?;
    if account.data.is_empty() {
        return Err(anyhow!("Bonding curve not found"));
    }

    // Use `deserialize` instead of `try_from_slice` so that extra trailing bytes
    // (from on-chain schema additions like new fields) are silently ignored.
    // `try_from_slice` requires the entire slice to be consumed, causing
    // "Not all bytes read" when the account has been extended.
    let mut bonding_curve = BondingCurveAccount::deserialize(&mut &account.data[8..])
        .map_err(|e| anyhow::anyhow!("Failed to decode bonding curve account: {}", e))?;
    bonding_curve.account = bonding_curve_pda;

    Ok((Arc::new(bonding_curve), bonding_curve_pda))
}

#[cfg(test)]
mod tests {
    use super::{
        global_constants, phantom_default_creator_vault, pump_fun_fee_recipient_meta,
        reconcile_mayhem_mode_for_trade, resolve_creator_vault_for_ix,
        resolve_creator_vault_for_ix_with_fee_sharing, *,
    };
    use solana_sdk::pubkey::Pubkey;

    #[test]
    fn pumpfun_discriminators_are_8_bytes() {
        assert_eq!(BUY_DISCRIMINATOR.len(), 8);
        assert_eq!(BUY_EXACT_SOL_IN_DISCRIMINATOR.len(), 8);
        assert_eq!(SELL_DISCRIMINATOR.len(), 8);
        assert_eq!(BUY_V2_DISCRIMINATOR.len(), 8);
        assert_eq!(SELL_V2_DISCRIMINATOR.len(), 8);
        assert_eq!(BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR.len(), 8);
    }

    #[test]
    fn pumpfun_bonding_curve_and_v2_pda_differ_for_same_mint() {
        let mint = Pubkey::new_unique();
        let pda = get_bonding_curve_pda(&mint).unwrap();
        let pda_v2 = get_bonding_curve_v2_pda(&mint).unwrap();
        assert_ne!(pda, pda_v2, "bonding_curve and bonding_curve_v2 PDAs must differ");
    }

    #[test]
    fn pumpfun_creator_vault_pda_deterministic() {
        let creator = Pubkey::new_unique();
        let a = get_creator_vault_pda(&creator).unwrap();
        let b = get_creator_vault_pda(&creator).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn fee_sharing_config_pda_deterministic() {
        let mint = Pubkey::new_unique();
        let a = get_fee_sharing_config_pda(&mint).unwrap();
        let b = get_fee_sharing_config_pda(&mint).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn default_creator_yields_fixed_creator_vault() {
        let v = get_creator_vault_pda(&Pubkey::default()).unwrap();
        assert_eq!(
            v,
            phantom_default_creator_vault(),
            "phantom vault constant must match PDA(default creator)"
        );
    }

    #[test]
    fn resolve_uses_ix_vault_when_creator_borsh_is_default() {
        let mint = Pubkey::new_unique();
        let ix_vault = Pubkey::new_unique();
        let resolved = resolve_creator_vault_for_ix(&Pubkey::default(), ix_vault, &mint);
        assert_eq!(resolved, Some(ix_vault));
    }

    #[test]
    fn resolve_returns_none_when_creator_and_vault_missing() {
        let mint = Pubkey::new_unique();
        assert_eq!(
            resolve_creator_vault_for_ix(&Pubkey::default(), Pubkey::default(), &mint),
            None
        );
    }

    #[test]
    fn resolve_rejects_phantom_vault_when_creator_borsh_is_default() {
        let mint = Pubkey::new_unique();
        assert_eq!(
            resolve_creator_vault_for_ix(
                &Pubkey::default(),
                phantom_default_creator_vault(),
                &mint,
            ),
            None
        );
    }

    #[test]
    fn resolve_prefers_creator_pda_ix_vault_even_when_fee_sharing_hint_differs() {
        let creator = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let v_derived = get_creator_vault_pda(&creator).unwrap();
        let sharing_pk = get_fee_sharing_config_pda(&mint).unwrap();
        let vs = get_creator_vault_pda(&sharing_pk).unwrap();
        assert_ne!(v_derived, vs);
        let resolved =
            resolve_creator_vault_for_ix_with_fee_sharing(&creator, v_derived, &mint, Some(vs));
        assert_eq!(
            resolved,
            Some(v_derived),
            "observed ix uses PDA(creator); stale hint must not override → wrong vault / Anchor 2006"
        );
    }

    #[test]
    fn resolve_trusts_ix_fee_sharing_vault_when_creator_known() {
        let creator = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let sharing_pk = get_fee_sharing_config_pda(&mint).unwrap();
        let vs = get_creator_vault_pda(&sharing_pk).unwrap();
        let creator_vault = get_creator_vault_pda(&creator).unwrap();
        assert_ne!(vs, creator_vault);
        let resolved = resolve_creator_vault_for_ix_with_fee_sharing(&creator, vs, &mint, Some(vs));
        assert_eq!(
            resolved,
            Some(vs),
            "explicit ix creator_vault (e.g. fee-sharing #8) wins; no remap to PDA(creator)"
        );
    }

    #[test]
    fn resolve_remaps_phantom_vault_when_creator_known() {
        let creator = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let expected = get_creator_vault_pda(&creator).unwrap();
        assert_eq!(
            resolve_creator_vault_for_ix(&creator, phantom_default_creator_vault(), &mint),
            Some(expected)
        );
    }

    #[test]
    fn resolve_ix_vault_always_wins_when_non_default_even_if_creator_pda_differs() {
        let creator = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let v_derived = get_creator_vault_pda(&creator).unwrap();
        let ix_other = Pubkey::new_unique();
        assert_ne!(ix_other, v_derived);
        let resolved =
            resolve_creator_vault_for_ix_with_fee_sharing(&creator, ix_other, &mint, None);
        assert_eq!(resolved, Some(ix_other));
    }

    #[test]
    fn resolve_fee_sharing_ix_vault_used_as_is_even_without_hint() {
        let creator = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let v_derived = get_creator_vault_pda(&creator).unwrap();
        let sharing_pk = get_fee_sharing_config_pda(&mint).unwrap();
        let vs = get_creator_vault_pda(&sharing_pk).unwrap();
        assert_ne!(v_derived, vs);
        let resolved = resolve_creator_vault_for_ix_with_fee_sharing(&creator, vs, &mint, None);
        assert_eq!(resolved, Some(vs));
    }

    #[test]
    fn resolve_falls_back_to_fee_sharing_hint_when_ix_vault_placeholder() {
        let creator = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let sharing_pk = get_fee_sharing_config_pda(&mint).unwrap();
        let vs = get_creator_vault_pda(&sharing_pk).unwrap();
        let resolved = resolve_creator_vault_for_ix_with_fee_sharing(
            &creator,
            Pubkey::default(),
            &mint,
            Some(vs),
        );
        assert_eq!(resolved, Some(vs));
    }

    #[test]
    fn reconcile_mayhem_prefers_fee_when_log_says_false_but_fee_is_mayhem_pool() {
        let fee = global_constants::MAYHEM_FEE_RECIPIENTS[0];
        assert!(reconcile_mayhem_mode_for_trade(Some(false), &fee));
    }

    #[test]
    fn reconcile_mayhem_prefers_fee_when_log_says_true_but_fee_is_standard_pool() {
        let fee = global_constants::PUMPFUN_AMM_FEE_4;
        assert!(!reconcile_mayhem_mode_for_trade(Some(true), &fee));
    }

    #[test]
    fn pump_fee_meta_rejects_standard_fee_when_building_mayhem_ix() {
        let fee = global_constants::PUMPFUN_AMM_FEE_4;
        let m = pump_fun_fee_recipient_meta(fee, true);
        assert!(
            global_constants::MAYHEM_FEE_RECIPIENTS.contains(&m.pubkey),
            "expected fallback to mayhem pool, got {}",
            m.pubkey
        );
    }

    #[test]
    fn pump_fee_meta_uses_observed_standard_fee_for_standard_ix() {
        let fee = global_constants::PUMPFUN_AMM_FEE_7;
        let m = pump_fun_fee_recipient_meta(fee, false);
        assert_eq!(m.pubkey, fee);
        assert!(m.is_writable);
        assert!(!m.is_signer);
    }
}
