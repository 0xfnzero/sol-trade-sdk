use crate::common::{
    fast_fn::create_associated_token_account_idempotent_fast,
    spl_token::close_account,
    seed::{create_associated_token_account_use_seed, get_associated_token_address_with_program_id_use_seed},
};
use smallvec::SmallVec;
use solana_sdk::{instruction::Instruction, message::AccountMeta, pubkey::Pubkey};
use solana_system_interface::instruction::transfer;

#[inline]
pub fn handle_wsol(payer: &Pubkey, amount_in: u64, is_use_seed: bool) -> SmallVec<[Instruction; 3]> {
    let wsol_token_account = if is_use_seed {
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
            &payer,
            &crate::constants::WSOL_TOKEN_ACCOUNT,
            &crate::constants::TOKEN_PROGRAM,
            true,
        )
    } else {
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &payer,
            &crate::constants::WSOL_TOKEN_ACCOUNT,
            &crate::constants::TOKEN_PROGRAM,
        )
    };

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

pub fn close_wsol(payer: &Pubkey, is_use_seed: bool) -> Vec<Instruction> {
    let wsol_token_account = if is_use_seed {
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
            &payer,
            &crate::constants::WSOL_TOKEN_ACCOUNT,
            &crate::constants::TOKEN_PROGRAM,
            true,
        )
    } else {
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &payer,
            &crate::constants::WSOL_TOKEN_ACCOUNT,
            &crate::constants::TOKEN_PROGRAM,
        )
    };
    crate::common::fast_fn::get_cached_instructions(
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
    )
}

#[inline]
pub fn create_wsol_ata(payer: &Pubkey, is_use_seed: bool) -> Vec<Instruction> {
    crate::common::fast_fn::create_associated_token_account_idempotent_fast_use_seed(
        &payer,
        &payer,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
        &crate::constants::TOKEN_PROGRAM,
        is_use_seed,
    )
}

/// 只充值SOL到已存在的WSOL ATA（不创建账户）- 标准方式
#[inline]
pub fn wrap_sol_only(payer: &Pubkey, amount_in: u64, is_use_seed: bool) -> SmallVec<[Instruction; 2]> {
    let wsol_token_account = if is_use_seed {
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
            &payer,
            &crate::constants::WSOL_TOKEN_ACCOUNT,
            &crate::constants::TOKEN_PROGRAM,
            true,
        )
    } else {
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &payer,
            &crate::constants::WSOL_TOKEN_ACCOUNT,
            &crate::constants::TOKEN_PROGRAM,
        )
    };

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

/// 将 WSOL 转换为 SOL，使用临时账户
/// 1. 创建临时 WSOL 账号（如果原始账号是seed创建，则临时账号使用普通ATA；否则使用seed）
/// 2. 获取临时账号的 ATA 地址
/// 3. 添加从用户 WSOL ATA 转账到临时 ATA 账号的指令
/// 4. 添加关闭临时 WSOL 账号的指令
/// 
/// # Arguments
/// * `payer` - 支付者公钥
/// * `amount` - 要转换的 WSOL 数量
/// * `source_is_seed` - 原始 WSOL 账号是否是 seed 创建的
pub fn wrap_wsol_to_sol(
    payer: &Pubkey,
    amount: u64,
    source_is_seed: bool,
) -> Result<Vec<Instruction>, anyhow::Error> {
    let mut instructions = Vec::new();

    // 1. 分别获取普通ATA和seed ATA地址
    let normal_ata = crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
        payer,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
        &crate::constants::TOKEN_PROGRAM,
    );
    let seed_ata = get_associated_token_address_with_program_id_use_seed(
        payer,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
        &crate::constants::TOKEN_PROGRAM,
    )?;

    // 2. 根据原始账号类型决定用户账号和临时账号
    // 如果原始账号是seed，则用户账号=seed_ata，临时账号=normal_ata（避免地址冲突）
    // 如果原始账号是普通ATA，则用户账号=normal_ata，临时账号=seed_ata（提高性能）
    let (user_wsol_ata, temp_ata_address, temp_account_instructions) = if source_is_seed {
        let create_insts = crate::common::fast_fn::create_associated_token_account_idempotent_fast(
            payer,
            payer,
            &crate::constants::WSOL_TOKEN_ACCOUNT,
            &crate::constants::TOKEN_PROGRAM,
        );
        (seed_ata, normal_ata, create_insts)
    } else {
        let create_insts = create_associated_token_account_use_seed(
            payer,
            payer,
            &crate::constants::WSOL_TOKEN_ACCOUNT,
            &crate::constants::TOKEN_PROGRAM,
        )?;
        (normal_ata, seed_ata, create_insts)
    };
    instructions.extend(temp_account_instructions);

    // 3. 添加从用户 WSOL ATA 转账到临时 ATA 的指令
    let transfer_instruction = crate::common::spl_token::transfer(
        &crate::constants::TOKEN_PROGRAM,
        &user_wsol_ata,
        &temp_ata_address,
        payer,
        amount,
        &[],
    )?;
    instructions.push(transfer_instruction);

    // 4. 添加关闭临时 WSOL 账户的指令
    let close_instruction = close_account(
        &crate::constants::TOKEN_PROGRAM,
        &temp_ata_address,
        payer,
        payer,
        &[],
    )?;
    instructions.push(close_instruction);

    Ok(instructions)
}
