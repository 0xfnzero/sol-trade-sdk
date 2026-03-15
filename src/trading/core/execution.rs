//! Execution: instruction preprocessing, cache prefetch, branch hints.
//! 执行模块：指令预处理、缓存预取、分支提示。

use anyhow::Result;
use solana_sdk::{instruction::Instruction, pubkey::Pubkey, signature::Keypair};

use crate::perf::{hardware_optimizations::BranchOptimizer, simd::SIMDMemory};

/// Solana account key size in bytes (Pubkey). 每个账户（Pubkey）的字节数。
pub const BYTES_PER_ACCOUNT: usize = 32;

/// Threshold above which we warn about large instruction count. 超过此次数会打 warning。
pub const MAX_INSTRUCTIONS_WARN: usize = 64;

/// Prefetch helper: triggers CPU prefetch for soon-to-be-accessed data to reduce cache-miss latency.
/// Call once on hot-path refs; no-op on non-x86_64. Safety: caller ensures valid read-only ref, no concurrent write.
/// 缓存预取：对即将访问的数据做 CPU 预取以降低 cache-miss；热路径上调用一次即可；非 x86_64 为 no-op。安全：调用方保证有效只读、无并发写。
pub struct Prefetch;

impl Prefetch {
    #[inline(always)]
    pub fn instructions(instructions: &[Instruction]) {
        if instructions.is_empty() {
            return;
        }
        // Prefetch first/middle/last instruction into L1 for subsequent build_transaction access. 预取首/中/尾指令到 L1。
        unsafe {
            BranchOptimizer::prefetch_read_data(&instructions[0]);
        }
        if instructions.len() > 2 {
            unsafe {
                BranchOptimizer::prefetch_read_data(&instructions[instructions.len() / 2]);
            }
        }
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

/// Memory operations (SIMD-accelerated where available). 内存操作（可用时走 SIMD 加速）。
pub struct MemoryOps;

impl MemoryOps {
    #[inline(always)]
    pub unsafe fn copy(dst: *mut u8, src: *const u8, len: usize) {
        SIMDMemory::copy_avx2(dst, src, len);
    }

    #[inline(always)]
    pub unsafe fn compare(a: *const u8, b: *const u8, len: usize) -> bool {
        SIMDMemory::compare_avx2(a, b, len)
    }

    #[inline(always)]
    pub unsafe fn zero(ptr: *mut u8, len: usize) {
        SIMDMemory::zero_avx2(ptr, len);
    }
}

/// Instruction preprocessing and validation. 指令预处理与校验。
pub struct InstructionProcessor;

impl InstructionProcessor {
    #[inline(always)]
    pub fn preprocess(instructions: &[Instruction]) -> Result<()> {
        if BranchOptimizer::unlikely(instructions.is_empty()) {
            return Err(anyhow::anyhow!("Instructions empty"));
        }

        Prefetch::instructions(instructions);

        if BranchOptimizer::unlikely(instructions.len() > MAX_INSTRUCTIONS_WARN) {
            tracing::warn!(target: "sol_trade_sdk", "Large instruction count: {}", instructions.len());
        }

        Ok(())
    }

    #[inline(always)]
    pub fn calculate_size(instructions: &[Instruction]) -> usize {
        let mut total_size = 0;

        for (i, instr) in instructions.iter().enumerate() {
            // Prefetch next instruction; safe: same slice, read-only. 预取下一条指令；安全：同 slice、只读。
            unsafe {
                if let Some(next_instr) = instructions.get(i + 1) {
                    BranchOptimizer::prefetch_read_data(next_instr);
                }
            }

            total_size += instr.data.len();
            total_size += instr.accounts.len() * BYTES_PER_ACCOUNT;
        }

        total_size
    }
}

/// Trade direction / execution path helpers. 交易方向与执行路径辅助。
pub struct ExecutionPath;

impl ExecutionPath {
    #[inline(always)]
    pub fn is_buy(input_mint: &Pubkey) -> bool {
        let is_buy = input_mint == &crate::constants::SOL_TOKEN_ACCOUNT
            || input_mint == &crate::constants::WSOL_TOKEN_ACCOUNT
            || input_mint == &crate::constants::USD1_TOKEN_ACCOUNT
            || input_mint == &crate::constants::USDC_TOKEN_ACCOUNT;

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
