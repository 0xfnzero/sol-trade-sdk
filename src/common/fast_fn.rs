use clru::CLruCache;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use std::num::NonZeroUsize;

use crate::common::{spl_associated_token_account::get_associated_token_address_with_program_id, spl_token::close_account};

const MAX_PDA_CACHE_SIZE: usize = 10000;
const MAX_ATA_CACHE_SIZE: usize = 10000;
const MAX_INSTRUCTION_CACHE_SIZE: usize = 10000;

// --------------------- Instruction Cache ---------------------

/// Instruction cache key for uniquely identifying instruction types and parameters
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum InstructionCacheKey {
    /// Associated Token Account creation instruction
    CreateAssociatedTokenAccount {
        payer: Pubkey,
        owner: Pubkey,
        mint: Pubkey,
        token_program: Pubkey,
        use_seed: bool,
    },
    /// Close wSOL Account
    CloseWsolAccount { payer: Pubkey, wsol_token_account: Pubkey },
}

/// Global instruction cache for storing common instructions
static INSTRUCTION_CACHE: Lazy<RwLock<CLruCache<InstructionCacheKey, Vec<Instruction>>>> =
    Lazy::new(|| {
        RwLock::new(CLruCache::new(NonZeroUsize::new(MAX_INSTRUCTION_CACHE_SIZE).unwrap()))
    });

/// Get cached instruction, compute and cache if not exists
pub fn get_cached_instructions<F>(cache_key: InstructionCacheKey, compute_fn: F) -> Vec<Instruction>
where
    F: FnOnce() -> Vec<Instruction>,
{
    // Try to get from cache (using read lock)
    {
        let cache = INSTRUCTION_CACHE.read();
        if let Some(cached_instruction) = cache.peek(&cache_key) {
            return cached_instruction.clone();
        }
    }

    // Cache miss, compute new instruction
    let instruction = compute_fn();

    // Store computation result in cache (using write lock)
    {
        let mut cache = INSTRUCTION_CACHE.write();
        cache.put(cache_key, instruction.clone());
    }

    instruction
}

// --------------------- Associated Token Account ---------------------

pub fn create_associated_token_account_idempotent_fast_use_seed(
    payer: &Pubkey,
    owner: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
    use_seed: bool,
) -> Vec<Instruction> {
    _create_associated_token_account_idempotent_fast(payer, owner, mint, token_program, use_seed)
}

pub fn create_associated_token_account_idempotent_fast(
    payer: &Pubkey,
    owner: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> Vec<Instruction> {
    _create_associated_token_account_idempotent_fast(payer, owner, mint, token_program, false)
}

pub fn _create_associated_token_account_idempotent_fast(
    payer: &Pubkey,
    owner: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
    use_seed: bool,
) -> Vec<Instruction> {
    // Create cache key
    let cache_key = InstructionCacheKey::CreateAssociatedTokenAccount {
        payer: *payer,
        owner: *owner,
        mint: *mint,
        token_program: *token_program,
        use_seed,
    };

    // Only use seed if the mint address is not wSOL or SOL
    // token 2022 测试不成功（TODO）
    if use_seed
        && !mint.eq(&crate::constants::WSOL_TOKEN_ACCOUNT)
        && !mint.eq(&crate::constants::SOL_TOKEN_ACCOUNT)
        && token_program.eq(&crate::constants::TOKEN_PROGRAM)
    {
        // Use cache to get instruction
        get_cached_instructions(cache_key, || {
            super::seed::create_associated_token_account_use_seed(payer, owner, mint, token_program)
                .unwrap()
        })
    } else {
        // Use cache to get instruction
        get_cached_instructions(cache_key, || {
            // Get Associated Token Address using cache
            let associated_token_address =
                get_associated_token_address_with_program_id_fast(owner, mint, token_program);
            // Create Associated Token Account instruction
            // Reference implementation of spl_associated_token_account::instruction::create_associated_token_account
            vec![Instruction {
                program_id: crate::constants::ASSOCIATED_TOKEN_PROGRAM_ID,
                accounts: vec![
                    AccountMeta::new(*payer, true), // Payer (signer, writable)
                    AccountMeta::new(associated_token_address, false), // ATA address (writable, non-signer)
                    AccountMeta::new_readonly(*owner, false), // Token account owner (readonly, non-signer)
                    AccountMeta::new_readonly(*mint, false), // Token mint address (readonly, non-signer)
                    crate::constants::SYSTEM_PROGRAM_META,
                    AccountMeta::new_readonly(*token_program, false), // Token program (readonly, non-signer)
                ],
                data: vec![1],
            }]
        })
    }
}

// --------------------- PDA ---------------------

/// PDA cache key for uniquely identifying PDA computation input parameters
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PdaCacheKey {
    PumpFunUserVolume(Pubkey),
    PumpFunBondingCurve(Pubkey),
    PumpFunCreatorVault(Pubkey),
    BonkPool(Pubkey, Pubkey),
    BonkVault(Pubkey, Pubkey),
    PumpSwapUserVolume(Pubkey),
}

/// Global PDA cache for storing computation results
static PDA_CACHE: Lazy<RwLock<CLruCache<PdaCacheKey, Pubkey>>> =
    Lazy::new(|| RwLock::new(CLruCache::new(NonZeroUsize::new(MAX_PDA_CACHE_SIZE).unwrap())));

/// Get cached PDA, compute and cache if not exists
pub fn get_cached_pda<F>(cache_key: PdaCacheKey, compute_fn: F) -> Option<Pubkey>
where
    F: FnOnce() -> Option<Pubkey>,
{
    // Try to get from cache (using read lock)
    {
        let cache = PDA_CACHE.read();
        if let Some(cached_pda) = cache.peek(&cache_key) {
            return Some(*cached_pda);
        }
    }

    // Cache miss, compute new PDA
    let pda_result = compute_fn();

    // If computation succeeds, store result in cache (using write lock)
    if let Some(pda) = pda_result {
        let mut cache = PDA_CACHE.write();
        cache.put(cache_key, pda);
    }

    pda_result
}

// --------------------- ATA ---------------------

/// ATA cache key for Associated Token Address caching
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct AtaCacheKey {
    wallet_address: Pubkey,
    token_mint_address: Pubkey,
    token_program_id: Pubkey,
    use_seed: bool,
}

/// Global ATA cache for storing Associated Token Address computation results
static ATA_CACHE: Lazy<RwLock<CLruCache<AtaCacheKey, Pubkey>>> =
    Lazy::new(|| RwLock::new(CLruCache::new(NonZeroUsize::new(MAX_ATA_CACHE_SIZE).unwrap())));

pub fn get_associated_token_address_with_program_id_fast_use_seed(
    wallet_address: &Pubkey,
    token_mint_address: &Pubkey,
    token_program_id: &Pubkey,
    use_seed: bool,
) -> Pubkey {
    _get_associated_token_address_with_program_id_fast(
        wallet_address,
        token_mint_address,
        token_program_id,
        use_seed,
    )
}

/// Get cached Associated Token Address, compute and cache if not exists
pub fn get_associated_token_address_with_program_id_fast(
    wallet_address: &Pubkey,
    token_mint_address: &Pubkey,
    token_program_id: &Pubkey,
) -> Pubkey {
    _get_associated_token_address_with_program_id_fast(
        wallet_address,
        token_mint_address,
        token_program_id,
        false,
    )
}

fn _get_associated_token_address_with_program_id_fast(
    wallet_address: &Pubkey,
    token_mint_address: &Pubkey,
    token_program_id: &Pubkey,
    use_seed: bool,
) -> Pubkey {
    let cache_key = AtaCacheKey {
        wallet_address: *wallet_address,
        token_mint_address: *token_mint_address,
        token_program_id: *token_program_id,
        use_seed,
    };

    // Try to get from cache (using read lock)
    {
        let cache = ATA_CACHE.read();
        if let Some(cached_ata) = cache.peek(&cache_key) {
            return *cached_ata;
        }
    }

    // Cache miss, compute new ATA
    // Only use seed if the token mint address is not wSOL or SOL
    // token 2022 测试不成功（TODO）
    let ata = if use_seed
        && !token_mint_address.eq(&crate::constants::WSOL_TOKEN_ACCOUNT)
        && !token_mint_address.eq(&crate::constants::SOL_TOKEN_ACCOUNT)
        && token_program_id.eq(&crate::constants::TOKEN_PROGRAM)
    {
        super::seed::get_associated_token_address_with_program_id_use_seed(
            wallet_address,
            token_mint_address,
            token_program_id,
        )
        .unwrap()
    } else {
        get_associated_token_address_with_program_id(
            wallet_address,
            token_mint_address,
            token_program_id,
        )
    };

    // Store computation result in cache (using write lock)
    {
        let mut cache = ATA_CACHE.write();
        cache.put(cache_key, ata);
    }

    ata
}

// --------------------- Initialize Accounts ---------------------

pub fn fast_init(payer: &Pubkey) {
    // Get PumpFun user volume accumulator PDA
    crate::instruction::utils::pumpfun::get_user_volume_accumulator_pda(payer);
    // Get PumpSwap user volume accumulator PDA
    crate::instruction::utils::pumpswap::get_user_volume_accumulator_pda(payer);
    // Get wSOL ATA address
    let wsol_token_account = get_associated_token_address_with_program_id_fast(
        payer,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
        &crate::constants::TOKEN_PROGRAM,
    );
    // Get Close wSOL Account instruction
    get_cached_instructions(
        crate::common::fast_fn::InstructionCacheKey::CloseWsolAccount {
            payer: *payer,
            wsol_token_account,
        },
        || {
            vec![close_account(
                &crate::constants::TOKEN_PROGRAM,
                &wsol_token_account,
                &payer,
                &payer,
                &[],
            )
            .unwrap()]
        },
    );
}
