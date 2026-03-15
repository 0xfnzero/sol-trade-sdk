//! Hardware-oriented optimizations: cache-line alignment, prefetch, SIMD, branch hints, memory barriers.
//! 硬件级优化：缓存行对齐与预取、SIMD、分支提示、内存屏障。

use anyhow::Result;
use crossbeam_utils::CachePadded;
use std::mem::size_of;
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};

/// Typical CPU cache line size in bytes. 典型 CPU 缓存行大小（字节）。
pub const CACHE_LINE_SIZE: usize = 64;

/// Trait for cache-line-aligned data and prefetch. 缓存行对齐与预取 trait。
pub trait CacheLineAligned {
    fn ensure_cache_aligned(&self) -> bool;
    fn prefetch_data(&self);
}

/// SIMD-accelerated memory operations. SIMD 加速的内存操作。
pub struct SIMDMemoryOps;

impl SIMDMemoryOps {
    /// SIMD-optimized copy by size class. 按长度分派的 SIMD 拷贝。
    #[inline(always)]
    pub unsafe fn memcpy_simd_optimized(dst: *mut u8, src: *const u8, len: usize) {
        match len {
            0 => return,
            1..=8 => Self::memcpy_small(dst, src, len),
            9..=16 => Self::memcpy_sse(dst, src, len),
            17..=32 => Self::memcpy_avx(dst, src, len),
            33..=64 => Self::memcpy_avx2(dst, src, len),
            _ => Self::memcpy_avx512_or_fallback(dst, src, len),
        }
    }

    /// Copy 1–8 bytes (scalar / small word). 小数据拷贝（1–8 字节）。
    #[inline(always)]
    unsafe fn memcpy_small(dst: *mut u8, src: *const u8, len: usize) {
        match len {
            1 => *dst = *src,
            2 => *(dst as *mut u16) = *(src as *const u16),
            3 => {
                *(dst as *mut u16) = *(src as *const u16);
                *dst.add(2) = *src.add(2);
            }
            4 => *(dst as *mut u32) = *(src as *const u32),
            5..=8 => {
                *(dst as *mut u64) = *(src as *const u64);
                if len > 8 {
                    ptr::copy_nonoverlapping(src.add(8), dst.add(8), len - 8);
                }
            }
            _ => unreachable!(),
        }
    }

    /// Copy 9–16 bytes using SSE (128-bit). SSE 拷贝（9–16 字节）。
    #[inline(always)]
    unsafe fn memcpy_sse(dst: *mut u8, src: *const u8, len: usize) {
        #[cfg(target_arch = "x86_64")]
        {
            use std::arch::x86_64::{__m128i, _mm_loadu_si128, _mm_storeu_si128};

            if len <= 16 {
                let chunk = _mm_loadu_si128(src as *const __m128i);
                _mm_storeu_si128(dst as *mut __m128i, chunk);
            }
        }

        #[cfg(not(target_arch = "x86_64"))]
        {
            ptr::copy_nonoverlapping(src, dst, len);
        }
    }

    /// Copy 17–32 bytes using AVX (256-bit). AVX 拷贝（17–32 字节）。
    #[inline(always)]
    unsafe fn memcpy_avx(dst: *mut u8, src: *const u8, len: usize) {
        #[cfg(target_arch = "x86_64")]
        {
            use std::arch::x86_64::{__m256i, _mm256_loadu_si256, _mm256_storeu_si256};

            if len <= 32 {
                let chunk = _mm256_loadu_si256(src as *const __m256i);
                _mm256_storeu_si256(dst as *mut __m256i, chunk);
            }
        }

        #[cfg(not(target_arch = "x86_64"))]
        {
            ptr::copy_nonoverlapping(src, dst, len);
        }
    }

    /// Copy 33–64 bytes using AVX2 (256-bit, two chunks). AVX2 拷贝（33–64 字节，两段）。
    #[inline(always)]
    unsafe fn memcpy_avx2(dst: *mut u8, src: *const u8, len: usize) {
        #[cfg(target_arch = "x86_64")]
        {
            use std::arch::x86_64::{__m256i, _mm256_loadu_si256, _mm256_storeu_si256};

            let chunk1 = _mm256_loadu_si256(src as *const __m256i);
            _mm256_storeu_si256(dst as *mut __m256i, chunk1);
            if len > 32 {
                let remaining = len - 32;
                if remaining <= 32 {
                    let chunk2 = _mm256_loadu_si256(src.add(32) as *const __m256i);
                    _mm256_storeu_si256(dst.add(32) as *mut __m256i, chunk2);
                }
            }
        }

        #[cfg(not(target_arch = "x86_64"))]
        {
            ptr::copy_nonoverlapping(src, dst, len);
        }
    }

    /// Copy >64 bytes: AVX-512 64-byte chunks when available, else AVX2 32-byte chunks. >64 字节：有 AVX512 用 64 字节块，否则 AVX2 32 字节块。
    #[inline(always)]
    unsafe fn memcpy_avx512_or_fallback(dst: *mut u8, src: *const u8, len: usize) {
        #[cfg(all(target_arch = "x86_64", target_feature = "avx512f"))]
        {
            use std::arch::x86_64::{__m512i, _mm512_loadu_si512, _mm512_storeu_si512};

            let chunks = len / 64;
            let mut offset = 0;

            for _ in 0..chunks {
                let chunk = _mm512_loadu_si512(src.add(offset) as *const __m512i);
                _mm512_storeu_si512(dst.add(offset) as *mut __m512i, chunk);
                offset += 64;
            }

            let remaining = len % 64;
            if remaining > 0 {
                Self::memcpy_avx2(dst.add(offset), src.add(offset), remaining);
            }
        }

        #[cfg(not(all(target_arch = "x86_64", target_feature = "avx512f")))]
        {
            let chunks = len / 32;
            let mut offset = 0;

            for _ in 0..chunks {
                Self::memcpy_avx2(dst.add(offset), src.add(offset), 32);
                offset += 32;
            }

            let remaining = len % 32;
            if remaining > 0 {
                Self::memcpy_avx(dst.add(offset), src.add(offset), remaining);
            }
        }
    }

    /// SIMD-optimized byte equality; dispatches by length (small / SSE / AVX2 / large). SIMD 加速的内存比较，按长度分派。
    #[inline(always)]
    pub unsafe fn memcmp_simd_optimized(a: *const u8, b: *const u8, len: usize) -> bool {
        match len {
            0 => true,
            1..=8 => Self::memcmp_small(a, b, len),
            9..=16 => Self::memcmp_sse(a, b, len),
            17..=32 => Self::memcmp_avx2(a, b, len),
            _ => Self::memcmp_large(a, b, len),
        }
    }

    /// Compare 1–8 bytes (scalar). 小数据比较（1–8 字节）。
    #[inline(always)]
    unsafe fn memcmp_small(a: *const u8, b: *const u8, len: usize) -> bool {
        match len {
            1 => *a == *b,
            2 => *(a as *const u16) == *(b as *const u16),
            3 => *(a as *const u16) == *(b as *const u16) && *a.add(2) == *b.add(2),
            4 => *(a as *const u32) == *(b as *const u32),
            5..=8 => *(a as *const u64) == *(b as *const u64),
            _ => unreachable!(),
        }
    }

    /// Compare 9–16 bytes using SSE. SSE 比较（9–16 字节）。
    #[inline(always)]
    unsafe fn memcmp_sse(a: *const u8, b: *const u8, len: usize) -> bool {
        #[cfg(target_arch = "x86_64")]
        {
            use std::arch::x86_64::{__m128i, _mm_cmpeq_epi8, _mm_loadu_si128, _mm_movemask_epi8};

            let chunk_a = _mm_loadu_si128(a as *const __m128i);
            let chunk_b = _mm_loadu_si128(b as *const __m128i);
            let cmp_result = _mm_cmpeq_epi8(chunk_a, chunk_b);
            let mask = _mm_movemask_epi8(cmp_result) as u32;

            let valid_mask = if len >= 16 { 0xFFFF } else { (1u32 << len) - 1 };
            (mask & valid_mask) == valid_mask
        }

        #[cfg(not(target_arch = "x86_64"))]
        {
            (0..len).all(|i| *a.add(i) == *b.add(i))
        }
    }

    /// Compare 17–32 bytes using AVX2. AVX2 比较（17–32 字节）。
    #[inline(always)]
    unsafe fn memcmp_avx2(a: *const u8, b: *const u8, len: usize) -> bool {
        #[cfg(target_arch = "x86_64")]
        {
            use std::arch::x86_64::{
                __m256i, _mm256_cmpeq_epi8, _mm256_loadu_si256, _mm256_movemask_epi8,
            };

            let chunk_a = _mm256_loadu_si256(a as *const __m256i);
            let chunk_b = _mm256_loadu_si256(b as *const __m256i);
            let cmp_result = _mm256_cmpeq_epi8(chunk_a, chunk_b);
            let mask = _mm256_movemask_epi8(cmp_result) as u32;

            let valid_mask = if len >= 32 { 0xFFFFFFFF } else { (1u32 << len) - 1 };
            (mask & valid_mask) == valid_mask
        }

        #[cfg(not(target_arch = "x86_64"))]
        {
            (0..len).all(|i| *a.add(i) == *b.add(i))
        }
    }

    /// Compare >32 bytes in 32-byte AVX2 chunks. 大数据比较（32 字节 AVX2 分块）。
    #[inline(always)]
    unsafe fn memcmp_large(a: *const u8, b: *const u8, len: usize) -> bool {
        let chunks = len / 32;

        for i in 0..chunks {
            let offset = i * 32;
            if !Self::memcmp_avx2(a.add(offset), b.add(offset), 32) {
                return false;
            }
        }

        let remaining = len % 32;
        if remaining > 0 {
            return Self::memcmp_avx2(a.add(chunks * 32), b.add(chunks * 32), remaining);
        }

        true
    }

    /// SIMD-optimized zero memory. SIMD 加速的内存清零。
    #[inline(always)]
    pub unsafe fn memzero_simd_optimized(ptr: *mut u8, len: usize) {
        #[cfg(target_arch = "x86_64")]
        {
            use std::arch::x86_64::{__m256i, _mm256_setzero_si256, _mm256_storeu_si256};

            let zero = _mm256_setzero_si256();
            let chunks = len / 32;
            let mut offset = 0;

            for _ in 0..chunks {
                _mm256_storeu_si256(ptr.add(offset) as *mut __m256i, zero);
                offset += 32;
            }

            let remaining = len % 32;
            for i in 0..remaining {
                *ptr.add(offset + i) = 0;
            }
        }

        #[cfg(not(target_arch = "x86_64"))]
        {
            ptr::write_bytes(ptr, 0, len);
        }
    }
}

/// Cache-line-aligned atomic counter. 缓存行对齐的原子计数器。
#[repr(align(64))]
pub struct CacheAlignedCounter {
    value: AtomicU64,
    _padding: [u8; CACHE_LINE_SIZE - size_of::<AtomicU64>()],
}

impl CacheAlignedCounter {
    /// Create counter with initial value. 创建并设置初值。
    pub fn new(initial: u64) -> Self {
        Self {
            value: AtomicU64::new(initial),
            _padding: [0; CACHE_LINE_SIZE - size_of::<AtomicU64>()],
        }
    }

    #[inline(always)]
    pub fn increment(&self) -> u64 {
        self.value.fetch_add(1, Ordering::Relaxed)
    }

    #[inline(always)]
    pub fn load(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }

    #[inline(always)]
    pub fn store(&self, val: u64) {
        self.value.store(val, Ordering::Relaxed)
    }
}

impl CacheLineAligned for CacheAlignedCounter {
    fn ensure_cache_aligned(&self) -> bool {
        (self as *const Self as usize) % CACHE_LINE_SIZE == 0
    }

    fn prefetch_data(&self) {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            use std::arch::x86_64::_mm_prefetch;
            use std::arch::x86_64::_MM_HINT_T0;
            _mm_prefetch(self as *const Self as *const i8, _MM_HINT_T0);
        }
    }
}

/// Cache-friendly lock-free ring buffer. 缓存友好的无锁环形缓冲区。
#[repr(align(64))]
pub struct CacheOptimizedRingBuffer<T> {
    buffer: Vec<T>,
    producer_head: CachePadded<AtomicU64>,
    consumer_tail: CachePadded<AtomicU64>,
    capacity: usize,
    mask: usize,
}

impl<T: Copy + Default> CacheOptimizedRingBuffer<T> {
    /// Create ring buffer; capacity must be a power of 2. 创建环形缓冲区，容量须为 2 的幂。
    pub fn new(capacity: usize) -> Result<Self> {
        if !capacity.is_power_of_two() {
            return Err(anyhow::anyhow!("Capacity must be a power of 2"));
        }

        let mut buffer = Vec::with_capacity(capacity);
        buffer.resize_with(capacity, Default::default);

        Ok(Self {
            buffer,
            producer_head: CachePadded::new(AtomicU64::new(0)),
            consumer_tail: CachePadded::new(AtomicU64::new(0)),
            capacity,
            mask: capacity - 1,
        })
    }

    /// Lock-free push; returns false if full. 无锁写入，满则返回 false。
    #[inline(always)]
    pub fn try_push(&self, item: T) -> bool {
        let current_head = self.producer_head.load(Ordering::Relaxed);
        let current_tail = self.consumer_tail.load(Ordering::Acquire);
        if (current_head + 1) & self.mask as u64 == current_tail & self.mask as u64 {
            return false;
        }
        unsafe {
            let index = current_head & self.mask as u64;
            let ptr = self.buffer.as_ptr().add(index as usize) as *mut T;
            ptr.write(item);
        }
        self.producer_head.store(current_head + 1, Ordering::Release);
        true
    }

    /// Lock-free pop; returns None if empty. 无锁读取，空则返回 None。
    #[inline(always)]
    pub fn try_pop(&self) -> Option<T> {
        let current_tail = self.consumer_tail.load(Ordering::Relaxed);
        let current_head = self.producer_head.load(Ordering::Acquire);
        if current_tail == current_head {
            return None;
        }
        let item = unsafe {
            let index = current_tail & self.mask as u64;
            let ptr = self.buffer.as_ptr().add(index as usize);
            ptr.read()
        };
        self.consumer_tail.store(current_tail + 1, Ordering::Release);
        Some(item)
    }

    /// Current number of elements. 当前元素个数。
    #[inline(always)]
    pub fn len(&self) -> usize {
        let head = self.producer_head.load(Ordering::Relaxed);
        let tail = self.consumer_tail.load(Ordering::Relaxed);
        ((head + self.capacity as u64 - tail) & self.mask as u64) as usize
    }

    /// True if no elements. 是否为空。
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.producer_head.load(Ordering::Relaxed) == self.consumer_tail.load(Ordering::Relaxed)
    }
}

impl<T> CacheLineAligned for CacheOptimizedRingBuffer<T> {
    fn ensure_cache_aligned(&self) -> bool {
        (self as *const Self as usize) % CACHE_LINE_SIZE == 0
    }

    fn prefetch_data(&self) {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            use std::arch::x86_64::_mm_prefetch;
            use std::arch::x86_64::_MM_HINT_T0;
            _mm_prefetch(self.producer_head.as_ptr() as *const i8, _MM_HINT_T0);
            _mm_prefetch(self.consumer_tail.as_ptr() as *const i8, _MM_HINT_T0);
            _mm_prefetch(self.buffer.as_ptr() as *const i8, _MM_HINT_T0);
        }
    }
}

/// Branch hint helpers (likely/unlikely) and prefetch. 分支提示与预取。
pub struct BranchOptimizer;

impl BranchOptimizer {
    /// Hint: condition is usually true. 提示编译器条件大概率为真。
    #[inline(always)]
    pub fn likely(condition: bool) -> bool {
        #[cold]
        fn cold() {}

        if !condition {
            cold();
        }
        condition
    }

    /// Hint: condition is usually false. 提示编译器条件大概率为假。
    #[inline(always)]
    pub fn unlikely(condition: bool) -> bool {
        #[cold]
        fn cold() {}

        if condition {
            cold();
        }
        condition
    }

    /// Prefetch: load cache line at ptr into L1. Caller must ensure ptr is valid, read-only, no concurrent write. 预取：将 ptr 所在缓存行加载到 L1；调用方需保证有效、只读、无并发写。
    #[inline(always)]
    pub unsafe fn prefetch_read_data<T>(ptr: *const T) {
        #[cfg(target_arch = "x86_64")]
        {
            use std::arch::x86_64::_mm_prefetch;
            use std::arch::x86_64::_MM_HINT_T0;
            _mm_prefetch(ptr as *const i8, _MM_HINT_T0);
        }
    }

    /// Prefetch for write (T1 hint). 写预取（T1 提示）。
    #[inline(always)]
    pub unsafe fn prefetch_write_data<T>(ptr: *const T) {
        #[cfg(target_arch = "x86_64")]
        {
            use std::arch::x86_64::_mm_prefetch;
            use std::arch::x86_64::_MM_HINT_T1;
            _mm_prefetch(ptr as *const i8, _MM_HINT_T1);
        }
    }
}

/// Memory barrier helpers. 内存屏障辅助。
pub struct MemoryBarriers;

impl MemoryBarriers {
    /// Compiler barrier only (no CPU reorder). 仅编译器屏障，防止重排序。
    #[inline(always)]
    pub fn compiler_barrier() {
        std::sync::atomic::compiler_fence(Ordering::SeqCst);
    }

    /// Light barrier (Acquire). 轻量级屏障（Acquire）。
    #[inline(always)]
    pub fn memory_barrier_light() {
        std::sync::atomic::fence(Ordering::Acquire);
    }

    /// Full sequential consistency barrier. 全序一致性屏障。
    #[inline(always)]
    pub fn memory_barrier_heavy() {
        std::sync::atomic::fence(Ordering::SeqCst);
    }

    /// Store/release barrier. 存储屏障，保证写入可见性。
    #[inline(always)]
    pub fn store_barrier() {
        std::sync::atomic::fence(Ordering::Release);
    }

    /// Load/acquire barrier. 加载屏障，保证读取顺序。
    #[inline(always)]
    pub fn load_barrier() {
        std::sync::atomic::fence(Ordering::Acquire);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_aligned_counter() {
        let counter = CacheAlignedCounter::new(0);
        assert!(counter.ensure_cache_aligned());

        assert_eq!(counter.load(), 0);
        counter.increment();
        assert_eq!(counter.load(), 1);
    }

    #[test]
    fn test_simd_memcpy() {
        let src = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let mut dst = [0u8; 10];

        unsafe {
            SIMDMemoryOps::memcpy_simd_optimized(dst.as_mut_ptr(), src.as_ptr(), src.len());
        }

        assert_eq!(src, dst);
    }

    #[test]
    fn test_cache_optimized_ring_buffer() {
        let buffer: CacheOptimizedRingBuffer<u64> = CacheOptimizedRingBuffer::new(16).unwrap();

        assert!(buffer.is_empty());

        // 测试推入
        assert!(buffer.try_push(42));
        assert_eq!(buffer.len(), 1);

        // 测试弹出
        assert_eq!(buffer.try_pop(), Some(42));
        assert!(buffer.is_empty());
    }

    #[test]
    fn test_simd_memcmp() {
        let a = [1u8, 2, 3, 4, 5];
        let b = [1u8, 2, 3, 4, 5];
        let c = [1u8, 2, 3, 4, 6];

        unsafe {
            assert!(SIMDMemoryOps::memcmp_simd_optimized(a.as_ptr(), b.as_ptr(), a.len()));

            assert!(!SIMDMemoryOps::memcmp_simd_optimized(a.as_ptr(), c.as_ptr(), a.len()));
        }
    }
}
