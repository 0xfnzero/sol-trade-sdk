//! 🚀 性能优化模块
//!
//! 提供多层次性能优化：
//! - SIMD 向量化：AVX2 内存操作、批量计算
//! - 硬件级优化：分支预测、缓存预取
//! - 零拷贝 I/O：内存映射、DMA传输
//! - 系统调用绕过：批处理、快速时间
//! - 编译器优化：内联、向量化

pub mod simd;
pub mod hardware_optimizations;
pub mod zero_copy_io;
pub mod syscall_bypass;
pub mod compiler_optimization;

pub use simd::*;
pub use hardware_optimizations::*;
pub use zero_copy_io::*;
pub use syscall_bypass::*;
pub use compiler_optimization::*;
