use crate::common::{
    fast_fn::create_associated_token_account_idempotent_fast,
    spl_token::close_account,
    seed::{create_associated_token_account_use_seed, get_associated_token_address_with_program_id_use_seed},
};
use smallvec::SmallVec;
use solana_sdk::{instruction::Instruction, message::AccountMeta, pubkey::Pubkey};
use solana_system_interface::instruction::transfer;

#[inline]
pub fn handle_wsol(payer: &Pubkey, amount_in: u64) -> SmallVec<[Instruction; 3]> {
    let wsol_token_account =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &payer,
            &crate::constants::WSOL_TOKEN_ACCOUNT,
            &crate::constants::TOKEN_PROGRAM,
        );

    let mut insts = SmallVec::<[Instruction; 3]>::new();
    insts.extend(create_associated_token_account_idempotent_fast(
        &payer,
        &payer,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
        &crate::constants::TOKEN_PROGRAM,
    ));
    insts.extend([
        transfer(&payer, &wsol_token_account, amount_in),
        // sync_native
        Instruction {
            program_id: crate::constants::TOKEN_PROGRAM,
            accounts: vec![AccountMeta::new(wsol_token_account, false)],
            data: vec![17],
        },
    ]);

    insts
}

pub fn close_wsol(payer: &Pubkey) -> Vec<Instruction> {
    use std::sync::Arc;
    
    let wsol_token_account =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &payer,
            &crate::constants::WSOL_TOKEN_ACCOUNT,
            &crate::constants::TOKEN_PROGRAM,
        );
    let arc_instructions = crate::common::fast_fn::get_cached_instructions(
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
    
    // 🚀 性能优化：尝试零开销解包 Arc
    Arc::try_unwrap(arc_instructions).unwrap_or_else(|arc| (*arc).clone())
}

#[inline]
pub fn create_wsol_ata(payer: &Pubkey) -> Vec<Instruction> {
    create_associated_token_account_idempotent_fast(
        &payer,
        &payer,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
        &crate::constants::TOKEN_PROGRAM,
    )
}

/// 只充值SOL到已存在的WSOL ATA（不创建账户）- 标准方式
#[inline]
pub fn wrap_sol_only(payer: &Pubkey, amount_in: u64) -> SmallVec<[Instruction; 2]> {
    let wsol_token_account =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &payer,
            &crate::constants::WSOL_TOKEN_ACCOUNT,
            &crate::constants::TOKEN_PROGRAM,
        );

    let mut insts = SmallVec::<[Instruction; 2]>::new();
    insts.extend([
        transfer(&payer, &wsol_token_account, amount_in),
        // sync_native
        Instruction {
            program_id: crate::constants::TOKEN_PROGRAM,
            accounts: vec![AccountMeta::new(wsol_token_account, false)],
            data: vec![17],
        },
    ]);

    insts
}

/// 将 WSOL 转换为 SOL，使用 seed 账户
/// 1. 检查 seed 账户是否已存在
/// 2. 如果不存在，使用 super::seed::create_associated_token_account_use_seed 创建 WSOL seed 账号
/// 3. 使用 get_associated_token_address_with_program_id_use_seed 获取该账号的 ATA 地址
/// 4. 添加从用户 WSOL ATA 转账到该 seed ATA 账号的指令
/// 5. 添加关闭 WSOL seed 账号的指令
///
/// 注意：此函数只生成指令，不检查账户是否存在（需要调用方在发送交易前检查）
/// 如果临时账户已存在，可以安全地跳过创建步骤，直接转账并关闭
pub fn wrap_wsol_to_sol(
    payer: &Pubkey,
    amount: u64,
) -> Result<Vec<Instruction>, anyhow::Error> {
    let mut instructions = Vec::new();

    // 1. 创建 WSOL seed 账户（注意：如果账户已存在会失败）
    // 调用方应该先检查账户是否存在，如果存在则跳过此步骤
    let seed_account_instructions = create_associated_token_account_use_seed(
        payer,
        payer,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
        &crate::constants::TOKEN_PROGRAM,
    )?;
    instructions.extend(seed_account_instructions);

    // 2. 获取 seed 账户的 ATA 地址
    let seed_ata_address = get_associated_token_address_with_program_id_use_seed(
        payer,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
        &crate::constants::TOKEN_PROGRAM,
    )?;

    // 3. 获取用户的 WSOL ATA 地址
    let user_wsol_ata = crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
        payer,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
        &crate::constants::TOKEN_PROGRAM,
    );

    // 4. 添加从用户 WSOL ATA 转账到 seed ATA 的指令
    let transfer_instruction = crate::common::spl_token::transfer(
        &crate::constants::TOKEN_PROGRAM,
        &user_wsol_ata,
        &seed_ata_address,
        payer,
        amount,
        &[],
    )?;
    instructions.push(transfer_instruction);

    // 5. 添加关闭 WSOL seed 账户的指令
    let close_instruction = close_account(
        &crate::constants::TOKEN_PROGRAM,
        &seed_ata_address,
        payer,
        payer,
        &[],
    )?;
    instructions.push(close_instruction);

    Ok(instructions)
}

/// 将 WSOL 转换为 SOL（仅转账和关闭，不创建账户）
/// 用于当临时seed账户已存在的情况
pub fn wrap_wsol_to_sol_without_create(
    payer: &Pubkey,
    amount: u64,
) -> Result<Vec<Instruction>, anyhow::Error> {
    let mut instructions = Vec::new();

    // 1. 获取 seed 账户的 ATA 地址
    let seed_ata_address = get_associated_token_address_with_program_id_use_seed(
        payer,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
        &crate::constants::TOKEN_PROGRAM,
    )?;

    // 2. 获取用户的 WSOL ATA 地址
    let user_wsol_ata = crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
        payer,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
        &crate::constants::TOKEN_PROGRAM,
    );

    // 3. 添加从用户 WSOL ATA 转账到 seed ATA 的指令
    let transfer_instruction = crate::common::spl_token::transfer(
        &crate::constants::TOKEN_PROGRAM,
        &user_wsol_ata,
        &seed_ata_address,
        payer,
        amount,
        &[],
    )?;
    instructions.push(transfer_instruction);

    // 4. 添加关闭 WSOL seed 账户的指令
    let close_instruction = close_account(
        &crate::constants::TOKEN_PROGRAM,
        &seed_ata_address,
        payer,
        payer,
        &[],
    )?;
    instructions.push(close_instruction);

    Ok(instructions)
}
