use solana_sdk::{compute_budget::ComputeBudgetInstruction, instruction::Instruction};

use crate::common::PriorityFee;

/// 为RPC交易添加计算预算指令
/// Add compute budget instructions for RPC transactions
pub fn add_rpc_compute_budget_instructions(
    instructions: &mut Vec<Instruction>,
    priority_fee: &PriorityFee,
    data_size_limit: u32,
) {
    instructions
        .push(ComputeBudgetInstruction::set_loaded_accounts_data_size_limit(data_size_limit));
    instructions.push(ComputeBudgetInstruction::set_compute_unit_price(
        priority_fee.rpc_unit_price,
    ));
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(
        priority_fee.rpc_unit_limit,
    ));
}

/// 为带小费的交易添加计算预算指令
/// Add compute budget instructions for transactions with tips
pub fn add_tip_compute_budget_instructions(
    instructions: &mut Vec<Instruction>,
    priority_fee: &PriorityFee,
    data_size_limit: u32,
) {
    instructions
        .push(ComputeBudgetInstruction::set_loaded_accounts_data_size_limit(data_size_limit));
    instructions.push(ComputeBudgetInstruction::set_compute_unit_price(
        priority_fee.unit_price,
    ));
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(
        priority_fee.unit_limit,
    ));
}

/// 通用的计算预算指令添加函数
/// Generic compute budget instruction addition function
pub fn add_compute_budget_instructions(
    instructions: &mut Vec<Instruction>,
    unit_price: u64,
    unit_limit: u32,
    data_size_limit: u32,
) {
    instructions
        .push(ComputeBudgetInstruction::set_loaded_accounts_data_size_limit(data_size_limit));
    instructions.push(ComputeBudgetInstruction::set_compute_unit_price(unit_price));
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(unit_limit));
}

pub fn add_sell_compute_budget_instructions(
    instructions: &mut Vec<Instruction>,
    priority_fee: &PriorityFee,
) {
    instructions.push(ComputeBudgetInstruction::set_compute_unit_price(
        priority_fee.rpc_unit_price,
    ));
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(
        priority_fee.rpc_unit_limit,
    ));
}

/// 为带小费的交易添加计算预算指令
/// Add compute budget instructions for transactions with tips
pub fn add_sell_tip_compute_budget_instructions(
    instructions: &mut Vec<Instruction>,
    priority_fee: &PriorityFee,
) {
    instructions.push(ComputeBudgetInstruction::set_compute_unit_price(
        priority_fee.unit_price,
    ));
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(
        priority_fee.unit_limit,
    ));
}
