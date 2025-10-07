//! 🚀 快速计时模块 - 减少 Instant::now() 系统调用开销
//!
//! 使用 syscall_bypass 提供的快速时间戳避免频繁的系统调用

use std::time::{Duration, Instant};
use once_cell::sync::Lazy;
use crate::perf::syscall_bypass::SystemCallBypassManager;

/// 全局快速时间提供器
static FAST_TIMER: Lazy<FastTimer> = Lazy::new(|| FastTimer::new());

/// 快速计时器 - 减少系统调用开销
pub struct FastTimer {
    bypass_manager: SystemCallBypassManager,
    _base_instant: Instant,
    _base_nanos: u64,
}

impl FastTimer {
    fn new() -> Self {
        use crate::perf::syscall_bypass::SyscallBypassConfig;

        let bypass_manager = SystemCallBypassManager::new(SyscallBypassConfig::default())
            .expect("Failed to create SystemCallBypassManager");

        let base_instant = Instant::now();
        let base_nanos = bypass_manager.fast_timestamp_nanos();

        Self {
            bypass_manager,
            _base_instant: base_instant,
            _base_nanos: base_nanos,
        }
    }

    /// 🚀 获取当前时间戳（纳秒） - 使用快速系统调用绕过
    #[inline(always)]
    pub fn now_nanos(&self) -> u64 {
        self.bypass_manager.fast_timestamp_nanos()
    }

    /// 🚀 获取当前时间戳（微秒）
    #[inline(always)]
    pub fn now_micros(&self) -> u64 {
        self.now_nanos() / 1_000
    }

    /// 🚀 获取当前时间戳（毫秒）
    #[inline(always)]
    pub fn now_millis(&self) -> u64 {
        self.now_nanos() / 1_000_000
    }

    /// 🚀 计算从开始到现在的耗时（纳秒）
    #[inline(always)]
    pub fn elapsed_nanos(&self, start_nanos: u64) -> u64 {
        self.now_nanos().saturating_sub(start_nanos)
    }

    /// 🚀 计算从开始到现在的耗时（Duration）
    #[inline(always)]
    pub fn elapsed_duration(&self, start_nanos: u64) -> Duration {
        Duration::from_nanos(self.elapsed_nanos(start_nanos))
    }
}

/// 🚀 快速获取当前时间戳（纳秒）- 全局函数
///
/// 使用 syscall_bypass 避免频繁的 clock_gettime 系统调用
#[inline(always)]
pub fn fast_now_nanos() -> u64 {
    FAST_TIMER.now_nanos()
}

/// 🚀 快速获取当前时间戳（微秒）
#[inline(always)]
pub fn fast_now_micros() -> u64 {
    FAST_TIMER.now_micros()
}

/// 🚀 快速获取当前时间戳（毫秒）
#[inline(always)]
pub fn fast_now_millis() -> u64 {
    FAST_TIMER.now_millis()
}

/// 🚀 计算耗时（纳秒）
#[inline(always)]
pub fn fast_elapsed_nanos(start_nanos: u64) -> u64 {
    FAST_TIMER.elapsed_nanos(start_nanos)
}

/// 🚀 计算耗时（Duration）
#[inline(always)]
pub fn fast_elapsed(start_nanos: u64) -> Duration {
    FAST_TIMER.elapsed_duration(start_nanos)
}

/// 快速计时器句柄 - 用于测量代码块耗时
pub struct FastStopwatch {
    start_nanos: u64,
    #[allow(dead_code)]
    label: &'static str,
}

impl FastStopwatch {
    /// 创建并启动计时器
    #[inline(always)]
    pub fn start(label: &'static str) -> Self {
        Self {
            start_nanos: fast_now_nanos(),
            label,
        }
    }

    /// 获取已耗时（纳秒）
    #[inline(always)]
    pub fn elapsed_nanos(&self) -> u64 {
        fast_elapsed_nanos(self.start_nanos)
    }

    /// 获取已耗时（Duration）
    #[inline(always)]
    pub fn elapsed(&self) -> Duration {
        fast_elapsed(self.start_nanos)
    }

    /// 获取已耗时（微秒）
    #[inline(always)]
    pub fn elapsed_micros(&self) -> u64 {
        self.elapsed_nanos() / 1_000
    }

    /// 获取已耗时（毫秒）
    #[inline(always)]
    pub fn elapsed_millis(&self) -> u64 {
        self.elapsed_nanos() / 1_000_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fast_timing() {
        let start = fast_now_nanos();
        std::thread::sleep(Duration::from_millis(10));
        let elapsed = fast_elapsed_nanos(start);

        // 应该大约是 10ms = 10,000,000 纳秒
        assert!(elapsed >= 9_000_000 && elapsed <= 12_000_000);
    }

    #[test]
    fn test_stopwatch() {
        let sw = FastStopwatch::start("test");
        std::thread::sleep(Duration::from_millis(10));
        let elapsed_ms = sw.elapsed_millis();

        assert!(elapsed_ms >= 9 && elapsed_ms <= 12);
    }

    #[test]
    fn test_fast_now_overhead() {
        // 测试调用开销
        let iterations = 10_000;
        let start = Instant::now();

        for _ in 0..iterations {
            let _ = fast_now_nanos();
        }

        let total_elapsed = start.elapsed();
        let avg_per_call = total_elapsed.as_nanos() / iterations;

        println!("Average fast_now_nanos() call: {}ns", avg_per_call);

        // 快速时间戳应该非常快（< 100ns per call）
        assert!(avg_per_call < 100);
    }

    #[test]
    fn test_instant_now_overhead() {
        // 对比标准 Instant::now() 的开销
        let iterations = 10_000;
        let start = Instant::now();

        for _ in 0..iterations {
            let _ = Instant::now();
        }

        let total_elapsed = start.elapsed();
        let avg_per_call = total_elapsed.as_nanos() / iterations;

        println!("Average Instant::now() call: {}ns", avg_per_call);
    }
}
