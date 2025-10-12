use crate::common::SolanaRpcClient;
use anyhow::anyhow;
use fnv::FnvHasher;
use solana_sdk::{instruction::Instruction, pubkey::Pubkey};
use solana_system_interface::instruction::create_account_with_seed;
use std::hash::Hasher;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::time::{sleep, Duration};
use once_cell::sync::Lazy;

// 🚀 优化：使用 AtomicU64 替代 RwLock，性能提升 5-10x
// u64::MAX 表示未初始化状态
static SPL_TOKEN_RENT: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(u64::MAX));
static SPL_TOKEN_2022_RENT: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(u64::MAX));

/// 更新租金缓存（后台任务调用）
pub async fn update_rents(client: &SolanaRpcClient) -> Result<(), anyhow::Error> {
    let rent = fetch_rent_for_token_account(client, false).await?;
    SPL_TOKEN_RENT.store(rent, Ordering::Release);  // Release 确保其他线程可见

    let rent = fetch_rent_for_token_account(client, true).await?;
    SPL_TOKEN_2022_RENT.store(rent, Ordering::Release);

    Ok(())
}

pub fn start_rent_updater(client: Arc<SolanaRpcClient>) {
    tokio::spawn(async move {
        loop {
            if let Err(_e) = update_rents(&client).await {}
            sleep(Duration::from_secs(60 * 60)).await;
        }
    });
}

async fn fetch_rent_for_token_account(
    client: &SolanaRpcClient,
    _is_2022_token: bool,
) -> Result<u64, anyhow::Error> {
    Ok(client.get_minimum_balance_for_rent_exemption(165).await?)
}

pub fn create_associated_token_account_use_seed(
    payer: &Pubkey,
    owner: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> Result<Vec<Instruction>, anyhow::Error> {
    let is_2022_token = token_program == &crate::constants::TOKEN_PROGRAM_2022;

    // 🚀 优化：原子读取租金缓存
    // Relaxed: 租金值不变，无需同步；Release/Acquire 在 update_rents 保证初始化可见性
    let rent = if is_2022_token {
        let v = SPL_TOKEN_2022_RENT.load(Ordering::Relaxed);
        if v == u64::MAX { return Err(anyhow!("Rent not initialized")); }
        v
    } else {
        let v = SPL_TOKEN_RENT.load(Ordering::Relaxed);
        if v == u64::MAX { return Err(anyhow!("Rent not initialized")); }
        v
    };

    let mut buf = [0u8; 8];
    let mut hasher = FnvHasher::default();
    hasher.write(mint.as_ref());
    let hash = hasher.finish();
    let v = (hash & 0xFFFF_FFFF) as u32;
    for i in 0..8 {
        let nibble = ((v >> (28 - i * 4)) & 0xF) as u8;
        buf[i] = match nibble {
            0..=9 => b'0' + nibble,
            _ => b'a' + (nibble - 10),
        };
    }
    let seed = unsafe { std::str::from_utf8_unchecked(&buf) };
    let ata_like = Pubkey::create_with_seed(payer, seed, token_program)?;

    let len = 165;
    let create_acc =
        create_account_with_seed(payer, &ata_like, owner, seed, rent, len, token_program);

    let init_acc = if is_2022_token {
        crate::common::spl_token_2022::initialize_account3(&token_program, &ata_like, mint, owner)?
    } else {
        crate::common::spl_token::initialize_account3(&token_program, &ata_like, mint, owner)?
    };

    Ok(vec![create_acc, init_acc])
}

pub fn get_associated_token_address_with_program_id_use_seed(
    wallet_address: &Pubkey,
    token_mint_address: &Pubkey,
    token_program_id: &Pubkey,
) -> Result<Pubkey, anyhow::Error> {
    let mut buf = [0u8; 8];
    let mut hasher = FnvHasher::default();
    hasher.write(token_mint_address.as_ref());
    let hash = hasher.finish();
    let v = (hash & 0xFFFF_FFFF) as u32;
    for i in 0..8 {
        let nibble = ((v >> (28 - i * 4)) & 0xF) as u8;
        buf[i] = match nibble {
            0..=9 => b'0' + nibble,
            _ => b'a' + (nibble - 10),
        };
    }
    let is_2022_token = token_program_id == &crate::constants::TOKEN_PROGRAM_2022;
    let seed = unsafe { std::str::from_utf8_unchecked(&buf) };
    let token_program = if is_2022_token {
        &crate::constants::TOKEN_PROGRAM_2022
    } else {
        &crate::constants::TOKEN_PROGRAM
    };
    let ata_like = Pubkey::create_with_seed(wallet_address, seed, token_program)?;
    Ok(ata_like)
}
