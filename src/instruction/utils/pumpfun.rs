use crate::common::{bonding_curve::BondingCurveAccount, SolanaRpcClient};
use anyhow::anyhow;
use rand::seq::IndexedRandom;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use std::sync::Arc;

// --- Aligned with official `@pump-fun/pump-sdk` (npm) ---
// - `src/fees.ts` `getFeeRecipient(global, mayhemMode)` — fee recipient pools
// - `src/bondingCurve.ts` `CURRENT_FEE_RECIPIENTS` / `getStaticRandomFeeRecipient`
// - `src/sdk.ts` `BONDING_CURVE_NEW_SIZE` (151) + `extendAccountInstruction` — **not** called from the
//   trade hot path here (no RPC in `PumpFunInstructionBuilder`); use these helpers from a cold path if needed.

/// Minimum bonding curve account data length after protocol upgrades (`sdk.ts` `BONDING_CURVE_NEW_SIZE`).
pub const PUMP_BONDING_CURVE_MIN_DATA_LEN: usize = 151;

/// Anchor discriminator for `extend_account` (`pump.json`); same as `PumpSdk.extendAccountInstruction`.
pub const EXTEND_ACCOUNT_DISCRIMINATOR: [u8; 8] = [234, 102, 194, 203, 150, 72, 62, 229];

/// Build `extend_account` for bonding curve (cold path / separate tx only — do not add RPC to hot-path builds).
#[inline]
pub fn extend_bonding_curve_account_instruction(bonding_curve: &Pubkey, user: &Pubkey) -> Instruction {
    Instruction {
        program_id: accounts::PUMPFUN,
        accounts: vec![
            AccountMeta::new(*bonding_curve, false),
            AccountMeta::new(*user, true),
            crate::constants::SYSTEM_PROGRAM_META,
            accounts::EVENT_AUTHORITY_META,
            accounts::PUMPFUN_META,
        ],
        data: EXTEND_ACCOUNT_DISCRIMINATOR.to_vec(),
    }
}

/// Constants used as seeds for deriving PDAs (Program Derived Addresses)
pub mod seeds {
    /// Seed for bonding curve PDAs
    pub const BONDING_CURVE_SEED: &[u8] = b"bonding-curve";
    /// Seed for bonding curve v2 PDA (required by program upgrade, readonly at end of account list)
    pub const BONDING_CURVE_V2_SEED: &[u8] = b"bonding-curve-v2";

    /// Seed for creator vault PDAs
    pub const CREATOR_VAULT_SEED: &[u8] = b"creator-vault";

    /// Seed for metadata PDAs
    pub const METADATA_SEED: &[u8] = b"metadata";

    /// Seed for user volume accumulator PDAs
    pub const USER_VOLUME_ACCUMULATOR_SEED: &[u8] = b"user_volume_accumulator";

    /// Seed for global volume accumulator PDAs
    pub const GLOBAL_VOLUME_ACCUMULATOR_SEED: &[u8] = b"global_volume_accumulator";

    pub const FEE_CONFIG_SEED: &[u8] = b"fee_config";

    /// `feeSharingConfig` PDA under pump-fees (`@pump-fun/pump-sdk` `feeSharingConfigPda`)
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

    pub const SCALE: u64 = 1_000_000; // 10^6 for token decimals

    pub const LAMPORTS_PER_SOL: u64 = 1_000_000_000; // 10^9 for solana lamports

    pub const COMPLETION_LAMPORTS: u64 = 85 * LAMPORTS_PER_SOL; // ~ 85 SOL

    /// Public key for the fee recipient
    pub const FEE_RECIPIENT: Pubkey = pubkey!("62qc2CNXwrYqQScmEdiZFFAnJR262PxWEuNQtxfafNgV");
    pub const FEE_RECIPIENT_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: FEE_RECIPIENT,
            is_signer: false,
            is_writable: true,
        };

    /// Mayhem fee recipients (pump-public-docs: use any one randomly)
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

    /// Public key for the global PDA
    pub const GLOBAL_ACCOUNT: Pubkey = pubkey!("4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf");
    pub const GLOBAL_ACCOUNT_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: GLOBAL_ACCOUNT,
            is_signer: false,
            is_writable: false,
        };

    /// Public key for the authority
    pub const AUTHORITY: Pubkey = pubkey!("FFWtrEQ4B4PKQoVuHYzZq8FabGkVatYzDpEVHsK5rrhF");

    /// Public key for the withdraw authority
    pub const WITHDRAW_AUTHORITY: Pubkey = pubkey!("39azUYFWPz3VHgKCf3VChUwbpURdCHRxjWVowf5jUJjg");

    pub const PUMPFUN_AMM_FEE_1: Pubkey = pubkey!("7VtfL8fvgNfhz17qKRMjzQEXgbdpnHHHQRh54R9jP2RJ"); // Pump.fun AMM: Protocol Fee 1
    pub const PUMPFUN_AMM_FEE_2: Pubkey = pubkey!("7hTckgnGnLQR6sdH7YkqFTAA7VwTfYFaZ6EhEsU3saCX"); // Pump.fun AMM: Protocol Fee 2
    pub const PUMPFUN_AMM_FEE_3: Pubkey = pubkey!("9rPYyANsfQZw3DnDmKE3YCQF5E8oD89UXoHn9JFEhJUz"); // Pump.fun AMM: Protocol Fee 3
    pub const PUMPFUN_AMM_FEE_4: Pubkey = pubkey!("AVmoTthdrX6tKt4nDjco2D775W2YK3sDhxPcMmzUAmTY"); // Pump.fun AMM: Protocol Fee 4
    pub const PUMPFUN_AMM_FEE_5: Pubkey = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM"); // Pump.fun AMM: Protocol Fee 5
    pub const PUMPFUN_AMM_FEE_6: Pubkey = pubkey!("FWsW1xNtWscwNmKv6wVsU1iTzRN6wmmk3MjxRP5tT7hz"); // Pump.fun AMM: Protocol Fee 6
    pub const PUMPFUN_AMM_FEE_7: Pubkey = pubkey!("G5UZAVbAf46s7cKWoyKu8kYTip9DGTpbLZ2qa9Aq69dP");
    // Pump.fun AMM: Protocol Fee 7

    /// Protocol extra fee recipients (Apr 2026 breaking upgrade). One is appended after `bonding-curve-v2`, **writable**.
    /// See: <https://github.com/pump-fun/pump-public-docs/blob/main/docs/BREAKING_FEE_RECIPIENT.md>
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
    /// Selected randomly from this pool — distinct from main fee recipients.
    pub const BUYBACK_FEE_RECIPIENTS: [Pubkey; 8] = [
        pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM"),
        pubkey!("FWsW1xNtWscwNmKv6wVsU1iTzRN6wmmk3MjxRP5tT7hz"),
        pubkey!("G5UZAVbAf46s7cKWoyKu8kYTip9DGTpbLZ2qa9Aq69dP"),
        pubkey!("AVmoTthdrX6tKt4nDjco2D775W2YK3sDhxPcMmzUAmTY"),
        pubkey!("9rPYyANsfQZw3DnDmKE3YCQF5E8oD89UXoHn9JFEhJUz"),
        pubkey!("7hTckgnGnLQR6sdH7YkqFTAA7VwTfYFaZ6EhEsU3saCX"),
        pubkey!("7VtfL8fvgNfhz17qKRMjzQEXgbdpnHHHQRh54R9jP2RJ"),
        pubkey!("8Wf5TiAheLUqBrKXeYg2JtAFFMWtKdG2BSFgqUcPVwTt"),
    ];
}

/// Constants related to program accounts and authorities
pub mod accounts {
    use solana_sdk::{pubkey, pubkey::Pubkey};

    /// Public key for the Pump.fun program
    pub const PUMPFUN: Pubkey = pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P");

    /// Public key for the MPL Token Metadata program
    pub const MPL_TOKEN_METADATA: Pubkey = pubkey!("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s");

    /// Authority for program events
    pub const EVENT_AUTHORITY: Pubkey = pubkey!("Ce6TQqeHC9p8KetsN6JsjHK7UTZk7nasjjnr7XxXp9F1");

    /// Associated Token Program ID
    pub const ASSOCIATED_TOKEN_PROGRAM: Pubkey =
        pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

    pub const AMM_PROGRAM: Pubkey = pubkey!("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8");

    pub const FEE_PROGRAM: Pubkey = pubkey!("pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ");

    pub const GLOBAL_VOLUME_ACCUMULATOR: Pubkey =
        pubkey!("Hq2wp8uJ9jCPsYgNHex8RtqdvMPfVGoYwjvF1ATiwn2Y");

    pub const FEE_CONFIG: Pubkey = pubkey!("8Wf5TiAheLUqBrKXeYg2JtAFFMWtKdG2BSFgqUcPVwTt");

    // META
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

/// Instruction discriminators for PumpFun program
pub const BUY_DISCRIMINATOR: [u8; 8] = [102, 6, 61, 18, 1, 218, 235, 234];
pub const BUY_EXACT_SOL_IN_DISCRIMINATOR: [u8; 8] = [56, 252, 116, 8, 158, 223, 205, 95];
pub const SELL_DISCRIMINATOR: [u8; 8] = [51, 230, 133, 164, 1, 127, 131, 173];
/// buy_v2: unified buy with quote_mint support (SOL + USDC), 27 fixed accounts
pub const BUY_V2_DISCRIMINATOR: [u8; 8] = [184, 23, 238, 97, 103, 197, 211, 61];
/// sell_v2: unified sell with quote_mint support (SOL + USDC), 26 fixed accounts
pub const SELL_V2_DISCRIMINATOR: [u8; 8] = [93, 246, 130, 60, 231, 233, 64, 178];
/// buy_exact_quote_in_v2: spend exact quote amount for min tokens out (SOL + USDC), 27 fixed accounts
pub const BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR: [u8; 8] = [194, 171, 28, 70, 104, 77, 91, 47];

/// Check if a pubkey is one of the Mayhem fee recipients
#[inline]
pub fn is_mayhem_fee_recipient(pubkey: &Pubkey) -> bool {
    global_constants::MAYHEM_FEE_RECIPIENTS.iter().any(|p| p == pubkey)
}

/// Check if a pubkey is a Pump.fun AMM protocol fee recipient (PUMPFUN_AMM_FEE_1..7)
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

/// Mayhem: random among `Global.reservedFeeRecipient` + `Global.reservedFeeRecipients` (`fees.ts` `getFeeRecipient` when `mayhemMode === true`).
/// Uses hardcoded `MAYHEM_FEE_RECIPIENTS`; prefer gRPC/event `PumpFunParams.fee_recipient` when set.
#[inline]
pub fn get_mayhem_fee_recipient_meta_random() -> AccountMeta {
    let recipient = *global_constants::MAYHEM_FEE_RECIPIENTS
        .choose(&mut rand::rng())
        .unwrap_or(&global_constants::MAYHEM_FEE_RECIPIENTS[0]);
    AccountMeta { pubkey: recipient, is_signer: false, is_writable: true }
}

/// Non-mayhem: random among `Global::fee_recipient` + `Global::fee_recipients[0..7]`.
/// Same pubkey set as `bondingCurve.ts` `CURRENT_FEE_RECIPIENTS` / `getStaticRandomFeeRecipient` and `fees.ts` `getFeeRecipient` when `mayhemMode === false`.
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
    let recipient = *POOL
        .choose(&mut rand::rng())
        .unwrap_or(&global_constants::FEE_RECIPIENT);
    AccountMeta {
        pubkey: recipient,
        is_signer: false,
        is_writable: true,
    }
}

/// Random entry from [`global_constants::PROTOCOL_EXTRA_FEE_RECIPIENTS`] (must be last account after bonding-curve-v2, writable).
#[inline]
pub fn get_protocol_extra_fee_recipient_random() -> Pubkey {
    *global_constants::PROTOCOL_EXTRA_FEE_RECIPIENTS
        .choose(&mut rand::rng())
        .unwrap_or(&global_constants::PROTOCOL_EXTRA_FEE_RECIPIENTS[0])
}

/// Resolve the effective quote_mint for a bonding curve.
///
/// - If `is_sol_paired` or `bonding_curve.quote_mint == Pubkey::default()` → wrapped SOL (`So1111...12`)
/// - Otherwise → the bonding curve's quote_mint (e.g. USDC)
#[inline]
pub fn resolve_quote_mint(
    _bonding_curve: &BondingCurveAccount,
    override_quote_mint: Option<Pubkey>,
) -> Pubkey {
    if let Some(qm) = override_quote_mint {
        if qm != Pubkey::default() {
            return qm;
        }
    }
    // Legacy bonding curves have no quote_mint field; default() means SOL-paired
    crate::constants::WSOL_TOKEN_ACCOUNT
}

/// Random buyback fee recipient from static pool (used as account #9 in buy_v2/sell_v2).
#[inline]
pub fn get_buyback_fee_recipient_random() -> Pubkey {
    *global_constants::BUYBACK_FEE_RECIPIENTS
        .choose(&mut rand::rng())
        .unwrap_or(&global_constants::BUYBACK_FEE_RECIPIENTS[0])
}

/// Get the quote token program for a given quote_mint.
/// SOL → Token Program; USDC → Token Program.
/// Both use the legacy SPL Token Program on mainnet.
#[inline]
pub fn get_quote_token_program(_quote_mint: &Pubkey) -> Pubkey {
    // Both WSOL and USDC use the legacy Token Program
    crate::constants::TOKEN_PROGRAM
}

/// 账户 #2 fee recipient：优先使用 gRPC/ShredStream 解析值（同笔 create_v2+buy 的 `observed_fee_recipient` 或 `tradeEvent.feeRecipient`）；未提供时按 mayhem 从静态池随机。
#[inline]
pub fn pump_fun_fee_recipient_meta(from_stream: Pubkey, is_mayhem_mode: bool) -> AccountMeta {
    if from_stream != Pubkey::default() {
        AccountMeta {
            pubkey: from_stream,
            is_signer: false,
            is_writable: true,
        }
    } else if is_mayhem_mode {
        get_mayhem_fee_recipient_meta_random()
    } else {
        get_standard_fee_recipient_meta_random()
    }
}

pub struct Symbol;

impl Symbol {
    pub const SOLANA: &'static str = "solana";
}

#[inline]
pub fn get_bonding_curve_pda(mint: &Pubkey) -> Option<Pubkey> {
    crate::common::fast_fn::get_cached_pda(
        crate::common::fast_fn::PdaCacheKey::PumpFunBondingCurve(*mint),
        || {
            let seeds: &[&[u8]; 2] = &[seeds::BONDING_CURVE_SEED, mint.as_ref()];
            let program_id: &Pubkey = &accounts::PUMPFUN;
            let pda: Option<(Pubkey, u8)> = Pubkey::try_find_program_address(seeds, program_id);
            pda.map(|pubkey| pubkey.0)
        },
    )
}

/// Bonding curve v2 PDA (seeds: ["bonding-curve-v2", mint]). Required at end of buy/sell/buy_exact_sol_in accounts.
#[inline]
pub fn get_bonding_curve_v2_pda(mint: &Pubkey) -> Option<Pubkey> {
    crate::common::fast_fn::get_cached_pda(
        crate::common::fast_fn::PdaCacheKey::PumpFunBondingCurveV2(*mint),
        || {
            let seeds: &[&[u8]; 2] = &[seeds::BONDING_CURVE_V2_SEED, mint.as_ref()];
            let program_id: &Pubkey = &accounts::PUMPFUN;
            let pda: Option<(Pubkey, u8)> = Pubkey::try_find_program_address(seeds, program_id);
            pda.map(|pubkey| pubkey.0)
        },
    )
}

#[inline]
pub fn get_creator(creator_vault_pda: &Pubkey) -> Pubkey {
    if creator_vault_pda.eq(&Pubkey::default()) {
        Pubkey::default()
    } else {
        // Fast check against cached default creator vault
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
            let pda: Option<(Pubkey, u8)> = Pubkey::try_find_program_address(seeds, program_id);
            pda.map(|pubkey| pubkey.0)
        },
    )
}

/// `feeSharingConfig` PDA per mint (`pump-sdk` `feeSharingConfigPda` → `pump-fees` program).
#[inline]
pub fn get_fee_sharing_config_pda(mint: &Pubkey) -> Option<Pubkey> {
    Pubkey::try_find_program_address(
        &[seeds::SHARING_CONFIG_SEED, mint.as_ref()],
        &accounts::FEE_PROGRAM,
    )
    .map(|(p, _)| p)
}

/// PDA of `["creator-vault", Pubkey::default()]`. Never use as a real vault — it is only produced when
/// `creator` was missing and code incorrectly derived a vault; on-chain this fails with Anchor 2006.
#[inline]
pub fn phantom_default_creator_vault() -> Pubkey {
    solana_sdk::pubkey!("2DR3iqRPVThyRLVJnwjPW1qiGWrp8RUFfHVjMbZyhdNc")
}

#[inline]
pub fn is_phantom_default_creator_vault(pk: &Pubkey) -> bool {
    *pk == phantom_default_creator_vault()
}

/// Resolve `creator_vault` for Pump buy/sell account #10.
///
/// - If `creator` is **missing** in the outer trade-event borsh (`Pubkey::default()`) but
///   `creator_vault` was filled from **instruction accounts** (e.g. `fill_trade_accounts` index 9),
///   **trust that vault** — unless it equals [`phantom_default_creator_vault`] (bad derivation / cache).
/// - If event `creator_vault` is **missing** → [`get_creator_vault_pda`]`(creator)` (never `PDA(default)`).
/// - If it **matches** `PDA(creator)` or `PDA(fee_sharing_config(mint))` → use it (fast path, matches ix).
/// - If it **does not match** either (e.g. stale vault but `creator` from tradeEvent is correct) → use
///   [`get_creator_vault_pda`]`(creator)` so seeds match on-chain bonding curve (fixes 2006 Left≠Right).
#[inline]
pub fn resolve_creator_vault_for_ix(
    creator: &Pubkey,
    creator_vault_from_event: Pubkey,
    mint: &Pubkey,
) -> Option<Pubkey> {
    let phantom = phantom_default_creator_vault();

    if *creator == Pubkey::default() {
        if creator_vault_from_event == Pubkey::default() {
            return None;
        }
        if creator_vault_from_event == phantom {
            return None;
        }
        return Some(creator_vault_from_event);
    }

    // Real creator: poisoned cache may hold phantom vault — always remap to PDA(creator).
    if creator_vault_from_event == phantom {
        return get_creator_vault_pda(creator);
    }

    let v_derived = get_creator_vault_pda(creator)?;
    if creator_vault_from_event == Pubkey::default() {
        return Some(v_derived);
    }
    if creator_vault_from_event == v_derived {
        return Some(creator_vault_from_event);
    }
    if let Some(sharing) = get_fee_sharing_config_pda(mint) {
        let v_sharing = get_creator_vault_pda(&sharing)?;
        if creator_vault_from_event == v_sharing {
            return Some(creator_vault_from_event);
        }
    }
    Some(v_derived)
}

#[inline]
pub fn get_user_volume_accumulator_pda(user: &Pubkey) -> Option<Pubkey> {
    crate::common::fast_fn::get_cached_pda(
        crate::common::fast_fn::PdaCacheKey::PumpFunUserVolume(*user),
        || {
            let seeds: &[&[u8]; 2] = &[seeds::USER_VOLUME_ACCUMULATOR_SEED, user.as_ref()];
            let program_id: &Pubkey = &accounts::PUMPFUN;
            let pda: Option<(Pubkey, u8)> = Pubkey::try_find_program_address(seeds, program_id);
            pda.map(|pubkey| pubkey.0)
        },
    )
}

#[inline]
pub async fn fetch_bonding_curve_account(
    rpc: &SolanaRpcClient,
    mint: &Pubkey,
) -> Result<(Arc<BondingCurveAccount>, Pubkey), anyhow::Error> {
    let bonding_curve_pda: Pubkey =
        get_bonding_curve_pda(mint).ok_or(anyhow!("Bonding curve not found"))?;

    let account = rpc.get_account(&bonding_curve_pda).await?;
    if account.data.is_empty() {
        return Err(anyhow!("Bonding curve not found"));
    }

    let bonding_curve =
        solana_sdk::borsh1::try_from_slice_unchecked::<BondingCurveAccount>(&account.data[8..])
            .map_err(|e| anyhow::anyhow!("Failed to deserialize bonding curve account: {}", e))?;

    Ok((Arc::new(bonding_curve), bonding_curve_pda))
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

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;

    #[test]
    fn pumpfun_discriminators_are_8_bytes() {
        assert_eq!(BUY_DISCRIMINATOR.len(), 8);
        assert_eq!(BUY_EXACT_SOL_IN_DISCRIMINATOR.len(), 8);
        assert_eq!(SELL_DISCRIMINATOR.len(), 8);
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
        assert_eq!(v, phantom_default_creator_vault(), "phantom vault constant must match PDA(default creator)");
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
            resolve_creator_vault_for_ix(&Pubkey::default(), phantom_default_creator_vault(), &mint),
            None
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
}
