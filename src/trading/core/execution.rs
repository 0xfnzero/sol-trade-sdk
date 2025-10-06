//! 执行模块

use anyhow::Result;
use solana_sdk::{
    instruction::Instruction,
    pubkey::Pubkey,
    signature::Keypair,
};

use crate::perf::{
    hardware_optimizations::BranchOptimizer,
    simd::SIMDMemory,
};

/// 预取工具
pub struct Prefetch;

impl Prefetch {
    #[inline(always)]
    pub fn instructions(instructions: &[Instruction]) {
        if instructions.is_empty() {
            return;
        }

        // 预取第一条指令
        unsafe {
            BranchOptimizer::prefetch_read_data(&instructions[0]);
        }

        // 预取中间指令
        if instructions.len() > 2 {
            unsafe {
                BranchOptimizer::prefetch_read_data(&instructions[instructions.len() / 2]);
            }
        }

        // 预取最后一条指令
        if instructions.len() > 1 {
            unsafe {
                BranchOptimizer::prefetch_read_data(&instructions[instructions.len() - 1]);
            }
        }
    }

    #[inline(always)]
    pub fn pubkey(pubkey: &Pubkey) {
        unsafe {
            BranchOptimizer::prefetch_read_data(pubkey);
        }
    }

    #[inline(always)]
    pub fn keypair(keypair: &Keypair) {
        unsafe {
            BranchOptimizer::prefetch_read_data(keypair);
        }
    }
}

/// 内存操作
pub struct MemoryOps;

impl MemoryOps {
    #[inline(always)]
    pub unsafe fn copy(dst: *mut u8, src: *const u8, len: usize) {
        // 优先使用 AVX2 SIMD 加速
        SIMDMemory::copy_avx2(dst, src, len);
    }

    #[inline(always)]
    pub unsafe fn compare(a: *const u8, b: *const u8, len: usize) -> bool {
        // 优先使用 AVX2 SIMD 比较
        SIMDMemory::compare_avx2(a, b, len)
    }

    #[inline(always)]
    pub unsafe fn zero(ptr: *mut u8, len: usize) {
        // 优先使用 AVX2 SIMD 清零
        SIMDMemory::zero_avx2(ptr, len);
    }
}

/// 指令处理器
pub struct InstructionProcessor;

impl InstructionProcessor {
    #[inline(always)]
    pub fn preprocess(instructions: &[Instruction]) -> Result<()> {
        // 分支预测: 大概率指令不为空
        if BranchOptimizer::unlikely(instructions.is_empty()) {
            return Err(anyhow::anyhow!("Instructions empty"));
        }

        // 预取所有指令到缓存
        Prefetch::instructions(instructions);

        // 分支预测: 大概率指令数量合理
        if BranchOptimizer::unlikely(instructions.len() > 64) {
            log::warn!("Large instruction count: {}", instructions.len());
        }

        Ok(())
    }

    #[inline(always)]
    pub fn calculate_size(instructions: &[Instruction]) -> usize {
        let mut total_size = 0;

        for instr in instructions {
            // 预取下一条指令
            unsafe {
                if let Some(next_instr) = instructions.get(total_size + 1) {
                    BranchOptimizer::prefetch_read_data(next_instr);
                }
            }

            total_size += instr.data.len();
            total_size += instr.accounts.len() * 32; // 每个账户 32 字节
        }

        total_size
    }
}

/// 执行路径
pub struct ExecutionPath;

impl ExecutionPath {
    #[inline(always)]
    pub fn is_buy(input_mint: &Pubkey) -> bool {
        // 分支预测: 大概率是买入
        let is_buy = input_mint == &crate::constants::SOL_TOKEN_ACCOUNT
            || input_mint == &crate::constants::WSOL_TOKEN_ACCOUNT
            || input_mint == &crate::constants::USD1_TOKEN_ACCOUNT;

        if BranchOptimizer::likely(is_buy) {
            return true;
        }

        false
    }

    #[inline(always)]
    pub fn select<T>(
        condition: bool,
        fast_path: impl FnOnce() -> T,
        slow_path: impl FnOnce() -> T,
    ) -> T {
        if BranchOptimizer::likely(condition) {
            fast_path()
        } else {
            slow_path()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::system_instruction;

    #[test]
    fn test_instruction_preprocessing() {
        let instructions = vec![
            system_instruction::transfer(
                &Pubkey::new_unique(),
                &Pubkey::new_unique(),
                1000,
            ),
        ];

        assert!(InstructionProcessor::preprocess(&instructions).is_ok());
    }

    #[test]
    fn test_memory_ops() {
        let src = vec![1u8, 2, 3, 4, 5];
        let mut dst = vec![0u8; 5];

        unsafe {
            MemoryOps::copy(dst.as_mut_ptr(), src.as_ptr(), src.len());
        }

        assert_eq!(src, dst);
    }
}
