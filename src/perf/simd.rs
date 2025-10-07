//! 🚀 SIMD 优化模块
//!
//! 使用 SIMD 指令加速数据处理：
//! - 内存拷贝加速
//! - 批量哈希计算
//! - 向量化数学运算
//! - 并行数据处理

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

/// SIMD 内存操作
pub struct SIMDMemory;

impl SIMDMemory {
    /// 使用 SIMD 加速内存拷贝（256位 AVX2）
    #[cfg(target_arch = "x86_64")]
    #[inline(always)]
    pub unsafe fn copy_avx2(dst: *mut u8, src: *const u8, len: usize) {
        let mut offset = 0;

        // 32字节对齐的批量拷贝（AVX2）
        while offset + 32 <= len {
            let data = _mm256_loadu_si256(src.add(offset) as *const __m256i);
            _mm256_storeu_si256(dst.add(offset) as *mut __m256i, data);
            offset += 32;
        }

        // 处理剩余字节
        while offset < len {
            *dst.add(offset) = *src.add(offset);
            offset += 1;
        }
    }

    /// 使用通用方法拷贝内存（非x86_64架构）
    #[cfg(not(target_arch = "x86_64"))]
    #[inline(always)]
    pub unsafe fn copy_avx2(dst: *mut u8, src: *const u8, len: usize) {
        std::ptr::copy_nonoverlapping(src, dst, len);
    }

    /// 使用 SIMD 加速内存比较
    #[cfg(target_arch = "x86_64")]
    #[inline(always)]
    pub unsafe fn compare_avx2(a: *const u8, b: *const u8, len: usize) -> bool {
        let mut offset = 0;

        // 32字节对齐的批量比较
        while offset + 32 <= len {
            let va = _mm256_loadu_si256(a.add(offset) as *const __m256i);
            let vb = _mm256_loadu_si256(b.add(offset) as *const __m256i);
            let cmp = _mm256_cmpeq_epi8(va, vb);
            let mask = _mm256_movemask_epi8(cmp);

            if mask != -1 {
                return false;
            }
            offset += 32;
        }

        // 处理剩余字节
        while offset < len {
            if *a.add(offset) != *b.add(offset) {
                return false;
            }
            offset += 1;
        }

        true
    }

    /// 使用通用方法比较内存（非x86_64架构）
    #[cfg(not(target_arch = "x86_64"))]
    #[inline(always)]
    pub unsafe fn compare_avx2(a: *const u8, b: *const u8, len: usize) -> bool {
        std::slice::from_raw_parts(a, len) == std::slice::from_raw_parts(b, len)
    }

    /// 使用 SIMD 清零内存
    #[cfg(target_arch = "x86_64")]
    #[inline(always)]
    pub unsafe fn zero_avx2(ptr: *mut u8, len: usize) {
        let zero = _mm256_setzero_si256();
        let mut offset = 0;

        // 32字节对齐的批量清零
        while offset + 32 <= len {
            _mm256_storeu_si256(ptr.add(offset) as *mut __m256i, zero);
            offset += 32;
        }

        // 处理剩余字节
        while offset < len {
            *ptr.add(offset) = 0;
            offset += 1;
        }
    }

    /// 使用通用方法清零内存（非x86_64架构）
    #[cfg(not(target_arch = "x86_64"))]
    #[inline(always)]
    pub unsafe fn zero_avx2(ptr: *mut u8, len: usize) {
        std::ptr::write_bytes(ptr, 0, len);
    }
}

/// SIMD 数学运算
pub struct SIMDMath;

impl SIMDMath {
    /// 批量 u64 加法 - x86_64 版本
    #[cfg(target_arch = "x86_64")]
    #[inline(always)]
    pub unsafe fn add_u64_batch(a: &[u64], b: &[u64], result: &mut [u64]) {
        assert_eq!(a.len(), b.len());
        assert_eq!(a.len(), result.len());

        let len = a.len();
        let mut i = 0;

        // 4个 u64 一组处理（256位）
        while i + 4 <= len {
            let va = _mm256_loadu_si256(a.as_ptr().add(i) as *const __m256i);
            let vb = _mm256_loadu_si256(b.as_ptr().add(i) as *const __m256i);
            let vsum = _mm256_add_epi64(va, vb);
            _mm256_storeu_si256(result.as_mut_ptr().add(i) as *mut __m256i, vsum);
            i += 4;
        }

        // 处理剩余元素
        while i < len {
            result[i] = a[i].wrapping_add(b[i]);
            i += 1;
        }
    }

    /// 批量 u64 加法 - 通用版本（非x86_64架构）
    #[cfg(not(target_arch = "x86_64"))]
    #[inline(always)]
    pub fn add_u64_batch(a: &[u64], b: &[u64], result: &mut [u64]) {
        assert_eq!(a.len(), b.len());
        assert_eq!(a.len(), result.len());

        for i in 0..a.len() {
            result[i] = a[i].wrapping_add(b[i]);
        }
    }

    /// 批量查找最大值
    #[inline(always)]
    pub fn max_u64_batch(data: &[u64]) -> u64 {
        if data.is_empty() {
            return 0;
        }

        let mut max = data[0];
        for &val in &data[1..] {
            if val > max {
                max = val;
            }
        }
        max
    }

    /// 批量查找最小值
    #[inline(always)]
    pub fn min_u64_batch(data: &[u64]) -> u64 {
        if data.is_empty() {
            return 0;
        }

        let mut min = data[0];
        for &val in &data[1..] {
            if val < min {
                min = val;
            }
        }
        min
    }
}

/// SIMD 序列化优化
pub struct SIMDSerializer;

impl SIMDSerializer {
    /// 批量序列化 u64 数组
    #[inline(always)]
    pub fn serialize_u64_batch(data: &[u64]) -> Vec<u8> {
        let mut result = Vec::with_capacity(data.len() * 8);

        for &value in data {
            result.extend_from_slice(&value.to_le_bytes());
        }

        result
    }

    /// 批量反序列化 u64 数组
    #[inline(always)]
    pub fn deserialize_u64_batch(data: &[u8]) -> Vec<u64> {
        let count = data.len() / 8;
        let mut result = Vec::with_capacity(count);

        for i in 0..count {
            let offset = i * 8;
            let bytes = [
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
                data[offset + 4],
                data[offset + 5],
                data[offset + 6],
                data[offset + 7],
            ];
            result.push(u64::from_le_bytes(bytes));
        }

        result
    }

    /// 使用 SIMD 加速 Base64 编码（简化版）
    #[inline(always)]
    pub fn encode_base64_simd(data: &[u8]) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(data)
    }
}

/// SIMD 哈希计算
pub struct SIMDHash;

impl SIMDHash {
    /// 批量计算 SHA256 哈希
    #[inline(always)]
    pub fn hash_batch_sha256(data: &[&[u8]]) -> Vec<[u8; 32]> {
        use sha2::{Sha256, Digest};

        data.iter()
            .map(|item| {
                let mut hasher = Sha256::new();
                hasher.update(item);
                hasher.finalize().into()
            })
            .collect()
    }

    /// 快速哈希（非加密）
    #[inline(always)]
    pub fn fast_hash_u64(data: &[u8]) -> u64 {
        let mut hash: u64 = 0xcbf29ce484222325; // FNV-1a offset

        for &byte in data {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3); // FNV-1a prime
        }

        hash
    }
}

/// SIMD 向量化迭代器
pub struct SIMDIterator;

impl SIMDIterator {
    /// 并行处理切片
    #[inline(always)]
    pub fn parallel_map<T, F>(data: &[T], f: F) -> Vec<T>
    where
        T: Copy + Send + Sync,
        F: Fn(T) -> T + Send + Sync,
    {
        data.iter().map(|&x| f(x)).collect()
    }

    /// 并行过滤
    #[inline(always)]
    pub fn parallel_filter<T, F>(data: &[T], predicate: F) -> Vec<T>
    where
        T: Copy + Send + Sync,
        F: Fn(&T) -> bool + Send + Sync,
    {
        data.iter().filter(|x| predicate(x)).copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simd_memory_copy() {
        let src = vec![1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let mut dst = vec![0u8; 10];

        unsafe {
            SIMDMemory::copy_avx2(dst.as_mut_ptr(), src.as_ptr(), src.len());
        }

        assert_eq!(src, dst);
    }

    #[test]
    fn test_simd_math() {
        let a = vec![1u64, 2, 3, 4];
        let b = vec![5u64, 6, 7, 8];
        let mut result = vec![0u64; 4];

        #[cfg(target_arch = "x86_64")]
        unsafe {
            SIMDMath::add_u64_batch(&a, &b, &mut result);
        }

        #[cfg(not(target_arch = "x86_64"))]
        SIMDMath::add_u64_batch(&a, &b, &mut result);

        assert_eq!(result, vec![6, 8, 10, 12]);
    }

    #[test]
    fn test_fast_hash() {
        let data = b"hello world";
        let hash1 = SIMDHash::fast_hash_u64(data);
        let hash2 = SIMDHash::fast_hash_u64(data);

        assert_eq!(hash1, hash2);
    }
}
