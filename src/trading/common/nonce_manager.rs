use anyhow::anyhow;
use solana_sdk::{
    instruction::Instruction,
    signature::Keypair,
    signer::Signer,
    system_instruction,
};
use solana_hash::Hash;

use crate::common::nonce_cache::NonceCache;

/// 添加nonce消费指令到指令集合中
/// Add nonce consumption instruction to the instruction set
///
/// 只有提供了nonce_pubkey时才使用nonce功能
/// Only use nonce functionality when nonce_pubkey is provided
/// 如果nonce被锁定、已使用或未准备好，将返回错误
/// If nonce is locked, used, or not ready, an error will be returned
/// 成功时会锁定并标记nonce为已使用
/// On success, it will lock and mark the nonce as used
pub fn add_nonce_instruction(
    instructions: &mut Vec<Instruction>, 
    payer: &Keypair
) -> Result<(), anyhow::Error> {
    let nonce_cache = NonceCache::get_instance();
    let nonce_info = nonce_cache.get_nonce_info();

    // 只检查nonce_account是否存在
    // Only check if nonce_account exists
    if let Some(nonce_pubkey) = nonce_info.nonce_account {
        // 暂不加锁
        // Temporarily not locking
        // if nonce_info.lock {
        //     return Err(anyhow!("Nonce is locked"));
        // }
        if nonce_info.used {
            return Err(anyhow!("Nonce is used"));
        }
        if nonce_info.current_nonce == Hash::default() {
            return Err(anyhow!("Nonce is not ready"));
        }
        // if nonce_info.next_buy_time == 0 || chrono::Utc::now().timestamp() < nonce_info.next_buy_time {
        //     return Err(anyhow!("Nonce is not ready"));
        // }
        // 加锁 - 暂不加锁
        // Lock - temporarily not locking
        // nonce_cache.lock();

        // 创建Solana系统nonce推进指令 - 使用系统程序ID
        // Create Solana system nonce advance instruction - using system program ID
        let nonce_advance_ix = system_instruction::advance_nonce_account(
            &nonce_pubkey,
            &payer.pubkey(),
        );

        instructions.push(nonce_advance_ix);
    }

    Ok(())
}

/// 获取用于交易的blockhash
/// Get blockhash for transaction
/// 如果使用了nonce账户，返回nonce中的blockhash，否则返回传入的recent_blockhash
/// If nonce account is used, return the blockhash in nonce, otherwise return the passed recent_blockhash
pub fn get_transaction_blockhash(recent_blockhash: Hash) -> Hash {
    let nonce_cache = NonceCache::get_instance();
    let nonce_info = nonce_cache.get_nonce_info();

    if nonce_info.nonce_account.is_some() {
        nonce_info.current_nonce
    } else {
        recent_blockhash
    }
}

/// 检查是否使用nonce账户
/// Check if nonce account is being used
pub fn is_using_nonce() -> bool {
    let nonce_cache = NonceCache::get_instance();
    let nonce_info = nonce_cache.get_nonce_info();
    nonce_info.nonce_account.is_some()
} 