//! Performance: SIMD, cache prefetch, branch hints, zero-copy I/O, syscall bypass, compiler hints.
//! 性能优化：SIMD、缓存预取、分支提示、零拷贝 I/O、系统调用绕过、编译器提示。

pub mod compiler_optimization;
pub mod hardware_optimizations;
pub mod simd;
pub mod syscall_bypass;
pub mod zero_copy_io;

pub use compiler_optimization::*;
pub use hardware_optimizations::*;
pub use simd::*;
pub use syscall_bypass::*;
pub use zero_copy_io::*;
