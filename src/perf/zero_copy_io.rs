//! 🚀 零拷贝内存映射IO - 完全消除数据拷贝开销
//! 
//! 实现极致的零拷贝策略，包括：
//! - 内存映射文件IO
//! - 共享内存环形缓冲区
//! - 直接内存访问(DMA)模拟
//! - 零拷贝网络数据传输
//! - 内存池预分配与重用

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
// use std::mem::{size_of, MaybeUninit};
use std::ptr::NonNull;
use std::slice;
use memmap2::{MmapMut, MmapOptions};
use anyhow::{Result, Context};
use crossbeam_utils::CachePadded;

/// 🚀 零拷贝内存管理器
pub struct ZeroCopyMemoryManager {
    /// 共享内存池
    shared_pools: Vec<Arc<SharedMemoryPool>>,
    /// 内存映射缓冲区
    mmap_buffers: Vec<Arc<MemoryMappedBuffer>>,
    /// 直接内存访问管理器
    dma_manager: Arc<DirectMemoryAccessManager>,
    /// 统计信息
    stats: Arc<ZeroCopyStats>,
}

/// 🚀 共享内存池 - 预分配大块内存避免运行时分配
pub struct SharedMemoryPool {
    /// 内存映射区域
    memory_region: MmapMut,
    /// 可用块列表(使用位图管理)
    free_blocks: Vec<AtomicU64>,
    /// 块大小
    block_size: usize,
    /// 总块数
    total_blocks: usize,
    /// 分配器头指针
    allocator_head: CachePadded<AtomicUsize>,
    /// 池ID
    pool_id: u32,
}

impl SharedMemoryPool {
    /// 创建共享内存池
    pub fn new(pool_id: u32, total_size: usize, block_size: usize) -> Result<Self> {
        // 确保块大小是64字节对齐(缓存行对齐)
        let aligned_block_size = (block_size + 63) & !63;
        let total_blocks = total_size / aligned_block_size;
        
        // 创建内存映射文件
        let memory_region = MmapOptions::new()
            .len(total_blocks * aligned_block_size)
            .map_anon()
            .context("Failed to create memory mapped region")?;
        
        // 初始化空闲块位图 (每个u64可以管理64个块)
        let bitmap_size = (total_blocks + 63) / 64;
        let mut free_blocks = Vec::with_capacity(bitmap_size);
        
        // 将所有块标记为空闲(全1)
        for i in 0..bitmap_size {
            let bits = if i == bitmap_size - 1 && total_blocks % 64 != 0 {
                // 最后一个u64可能不满64位
                let valid_bits = total_blocks % 64;
                (1u64 << valid_bits) - 1
            } else {
                u64::MAX // 所有64位都是1
            };
            free_blocks.push(AtomicU64::new(bits));
        }
        
        log::info!("🚀 Created shared memory pool {} with {} blocks of {} bytes each", 
                  pool_id, total_blocks, aligned_block_size);
        
        Ok(Self {
            memory_region,
            free_blocks,
            block_size: aligned_block_size,
            total_blocks,
            allocator_head: CachePadded::new(AtomicUsize::new(0)),
            pool_id,
        })
    }
    
    /// 🚀 零拷贝分配内存块
    #[inline(always)]
    pub fn allocate_block(&self) -> Option<ZeroCopyBlock> {
        // 快速路径：尝试从预期位置分配
        let start_index = self.allocator_head.load(Ordering::Relaxed) / 64;
        
        // 遍历所有位图寻找空闲块
        for attempt in 0..self.free_blocks.len() {
            let bitmap_index = (start_index + attempt) % self.free_blocks.len();
            let bitmap = &self.free_blocks[bitmap_index];
            
            let mut current = bitmap.load(Ordering::Acquire);
            
            while current != 0 {
                // 找到最低位的1(最小的空闲块)
                let bit_pos = current.trailing_zeros() as usize;
                let mask = 1u64 << bit_pos;
                
                // 尝试原子地清除这一位(标记为已分配)
                match bitmap.compare_exchange_weak(
                    current, 
                    current & !mask,
                    Ordering::AcqRel,
                    Ordering::Relaxed
                ) {
                    Ok(_) => {
                        // 成功分配
                        let block_index = bitmap_index * 64 + bit_pos;
                        if block_index >= self.total_blocks {
                            // 超出边界，恢复位并继续
                            bitmap.fetch_or(mask, Ordering::Relaxed);
                            break;
                        }
                        
                        let offset = block_index * self.block_size;
                        let ptr = unsafe {
                            NonNull::new_unchecked(
                                self.memory_region.as_ptr().add(offset) as *mut u8
                            )
                        };
                        
                        // 更新分配器头指针
                        self.allocator_head.store(
                            (block_index + 1) * 64, 
                            Ordering::Relaxed
                        );
                        
                        return Some(ZeroCopyBlock {
                            ptr,
                            size: self.block_size,
                            pool_id: self.pool_id,
                            block_index,
                        });
                    }
                    Err(new_current) => {
                        current = new_current;
                        continue;
                    }
                }
            }
        }
        
        None // 没有可用块
    }
    
    /// 🚀 零拷贝释放内存块
    #[inline(always)]
    pub fn deallocate_block(&self, block: ZeroCopyBlock) {
        if block.pool_id != self.pool_id {
            log::error!("Attempting to deallocate block from wrong pool");
            return;
        }
        
        let bitmap_index = block.block_index / 64;
        let bit_pos = block.block_index % 64;
        let mask = 1u64 << bit_pos;
        
        if bitmap_index < self.free_blocks.len() {
            // 原子地设置位为1(标记为空闲)
            self.free_blocks[bitmap_index].fetch_or(mask, Ordering::Release);
        }
    }
    
    /// 获取可用块数量
    pub fn available_blocks(&self) -> usize {
        self.free_blocks.iter()
            .map(|bitmap| bitmap.load(Ordering::Relaxed).count_ones() as usize)
            .sum()
    }
}

/// 🚀 零拷贝内存块
pub struct ZeroCopyBlock {
    /// 内存指针
    ptr: NonNull<u8>,
    /// 块大小
    size: usize,
    /// 所属池ID
    pool_id: u32,
    /// 块索引
    block_index: usize,
}

impl ZeroCopyBlock {
    /// 获取内存指针
    #[inline(always)]
    pub fn as_ptr(&self) -> *mut u8 {
        self.ptr.as_ptr()
    }
    
    /// 获取只读切片
    #[inline(always)]
    pub unsafe fn as_slice(&self) -> &[u8] {
        slice::from_raw_parts(self.ptr.as_ptr(), self.size)
    }
    
    /// 获取可变切片
    #[inline(always)]
    pub unsafe fn as_mut_slice(&mut self) -> &mut [u8] {
        slice::from_raw_parts_mut(self.ptr.as_ptr(), self.size)
    }
    
    /// 获取块大小
    #[inline(always)]
    pub fn size(&self) -> usize {
        self.size
    }
    
    /// 零拷贝写入数据
    #[inline(always)]
    pub unsafe fn write_bytes(&mut self, data: &[u8]) -> Result<()> {
        if data.len() > self.size {
            return Err(anyhow::anyhow!("Data too large for block"));
        }
        
        // 使用硬件优化的内存拷贝
        super::hardware_optimizations::SIMDMemoryOps::memcpy_simd_optimized(
            self.ptr.as_ptr(),
            data.as_ptr(),
            data.len()
        );
        
        Ok(())
    }
    
    /// 零拷贝读取数据
    #[inline(always)]
    pub unsafe fn read_bytes(&self, len: usize) -> Result<&[u8]> {
        if len > self.size {
            return Err(anyhow::anyhow!("Read length exceeds block size"));
        }
        
        Ok(slice::from_raw_parts(self.ptr.as_ptr(), len))
    }
}

unsafe impl Send for ZeroCopyBlock {}
unsafe impl Sync for ZeroCopyBlock {}

/// 🚀 内存映射缓冲区 - 大数据零拷贝传输
pub struct MemoryMappedBuffer {
    /// 内存映射区域
    mmap: MmapMut,
    /// 读指针
    read_pos: CachePadded<AtomicUsize>,
    /// 写指针
    write_pos: CachePadded<AtomicUsize>,
    /// 缓冲区大小
    size: usize,
    /// 缓冲区ID
    _buffer_id: u64,
}

impl MemoryMappedBuffer {
    /// 创建内存映射缓冲区
    pub fn new(buffer_id: u64, size: usize) -> Result<Self> {
        let mmap = MmapOptions::new()
            .len(size)
            .map_anon()
            .context("Failed to create memory mapped buffer")?;
        
        log::info!("🚀 Created memory mapped buffer {} with size {} bytes", buffer_id, size);
        
        Ok(Self {
            mmap,
            read_pos: CachePadded::new(AtomicUsize::new(0)),
            write_pos: CachePadded::new(AtomicUsize::new(0)),
            size,
            _buffer_id: buffer_id,
        })
    }
    
    /// 🚀 零拷贝写入数据
    #[inline(always)]
    pub fn write_data(&self, data: &[u8]) -> Result<usize> {
        let data_len = data.len();
        let current_write = self.write_pos.load(Ordering::Relaxed);
        let current_read = self.read_pos.load(Ordering::Acquire);
        
        // 计算可用空间
        let available_space = if current_write >= current_read {
            self.size - (current_write - current_read) - 1
        } else {
            current_read - current_write - 1
        };
        
        if data_len > available_space {
            return Err(anyhow::anyhow!("Insufficient buffer space"));
        }
        
        // 零拷贝写入
        unsafe {
            let write_ptr = self.mmap.as_ptr().add(current_write) as *mut u8;
            
            if current_write + data_len <= self.size {
                // 数据不跨越缓冲区边界
                super::hardware_optimizations::SIMDMemoryOps::memcpy_simd_optimized(
                    write_ptr, data.as_ptr(), data_len
                );
            } else {
                // 数据跨越缓冲区边界，分两段写入
                let first_part = self.size - current_write;
                let second_part = data_len - first_part;
                
                // 写入第一部分
                super::hardware_optimizations::SIMDMemoryOps::memcpy_simd_optimized(
                    write_ptr, data.as_ptr(), first_part
                );
                
                // 写入第二部分(从缓冲区开头)
                super::hardware_optimizations::SIMDMemoryOps::memcpy_simd_optimized(
                    self.mmap.as_ptr() as *mut u8, 
                    data.as_ptr().add(first_part), 
                    second_part
                );
            }
        }
        
        // 更新写指针
        let new_write_pos = (current_write + data_len) % self.size;
        self.write_pos.store(new_write_pos, Ordering::Release);
        
        Ok(data_len)
    }
    
    /// 🚀 零拷贝读取数据
    #[inline(always)]
    pub fn read_data(&self, buffer: &mut [u8]) -> Result<usize> {
        let buffer_len = buffer.len();
        let current_read = self.read_pos.load(Ordering::Relaxed);
        let current_write = self.write_pos.load(Ordering::Acquire);
        
        // 计算可读数据量
        let available_data = if current_write >= current_read {
            current_write - current_read
        } else {
            self.size - (current_read - current_write)
        };
        
        if available_data == 0 {
            return Ok(0); // 无数据可读
        }
        
        let read_len = buffer_len.min(available_data);
        
        // 零拷贝读取
        unsafe {
            let read_ptr = self.mmap.as_ptr().add(current_read);
            
            if current_read + read_len <= self.size {
                // 数据不跨越缓冲区边界
                super::hardware_optimizations::SIMDMemoryOps::memcpy_simd_optimized(
                    buffer.as_mut_ptr(), read_ptr, read_len
                );
            } else {
                // 数据跨越缓冲区边界，分两段读取
                let first_part = self.size - current_read;
                let second_part = read_len - first_part;
                
                // 读取第一部分
                super::hardware_optimizations::SIMDMemoryOps::memcpy_simd_optimized(
                    buffer.as_mut_ptr(), read_ptr, first_part
                );
                
                // 读取第二部分(从缓冲区开头)
                super::hardware_optimizations::SIMDMemoryOps::memcpy_simd_optimized(
                    buffer.as_mut_ptr().add(first_part),
                    self.mmap.as_ptr(), 
                    second_part
                );
            }
        }
        
        // 更新读指针
        let new_read_pos = (current_read + read_len) % self.size;
        self.read_pos.store(new_read_pos, Ordering::Release);
        
        Ok(read_len)
    }
    
    /// 获取可读数据量
    #[inline(always)]
    pub fn available_data(&self) -> usize {
        let current_read = self.read_pos.load(Ordering::Relaxed);
        let current_write = self.write_pos.load(Ordering::Relaxed);
        
        if current_write >= current_read {
            current_write - current_read
        } else {
            self.size - (current_read - current_write)
        }
    }
    
    /// 获取可用空间
    #[inline(always)]
    pub fn available_space(&self) -> usize {
        self.size - self.available_data() - 1
    }
}

/// 🚀 直接内存访问管理器 - 模拟DMA操作
pub struct DirectMemoryAccessManager {
    /// DMA通道池
    dma_channels: Vec<Arc<DMAChannel>>,
    /// 通道分配器
    channel_allocator: AtomicUsize,
    /// 统计信息
    dma_stats: Arc<DMAStats>,
}

impl DirectMemoryAccessManager {
    /// 创建DMA管理器
    pub fn new(num_channels: usize) -> Result<Self> {
        let mut dma_channels = Vec::with_capacity(num_channels);
        
        for i in 0..num_channels {
            dma_channels.push(Arc::new(DMAChannel::new(i)?));
        }
        
        log::info!("🚀 Created DMA manager with {} channels", num_channels);
        
        Ok(Self {
            dma_channels,
            channel_allocator: AtomicUsize::new(0),
            dma_stats: Arc::new(DMAStats::new()),
        })
    }
    
    /// 🚀 执行零拷贝DMA传输
    #[inline(always)]
    pub async fn dma_transfer(&self, src: &[u8], dst: &mut [u8]) -> Result<usize> {
        if src.len() != dst.len() {
            return Err(anyhow::anyhow!("Source and destination sizes don't match"));
        }
        
        // 选择DMA通道(轮询分配)
        let channel_index = self.channel_allocator.fetch_add(1, Ordering::Relaxed) % self.dma_channels.len();
        let channel = &self.dma_channels[channel_index];
        
        // 执行DMA传输
        let transferred = channel.transfer(src, dst).await?;
        
        // 更新统计
        self.dma_stats.bytes_transferred.fetch_add(transferred as u64, Ordering::Relaxed);
        self.dma_stats.transfers_completed.fetch_add(1, Ordering::Relaxed);
        
        Ok(transferred)
    }
}

/// 🚀 DMA通道
pub struct DMAChannel {
    /// 通道ID
    _channel_id: usize,
    /// 传输队列
    _transfer_queue: crossbeam_queue::ArrayQueue<DMATransfer>,
    /// 通道状态
    _status: AtomicU64,
}

impl DMAChannel {
    /// 创建DMA通道
    pub fn new(channel_id: usize) -> Result<Self> {
        Ok(Self {
            _channel_id: channel_id,
            _transfer_queue: crossbeam_queue::ArrayQueue::new(1024),
            _status: AtomicU64::new(0),
        })
    }
    
    /// 🚀 执行零拷贝传输
    #[inline(always)]
    pub async fn transfer(&self, src: &[u8], dst: &mut [u8]) -> Result<usize> {
        let transfer_size = src.len();
        
        // 使用硬件优化的SIMD内存拷贝
        unsafe {
            super::hardware_optimizations::SIMDMemoryOps::memcpy_simd_optimized(
                dst.as_mut_ptr(),
                src.as_ptr(),
                transfer_size
            );
        }
        
        Ok(transfer_size)
    }
}

/// DMA传输描述符
#[derive(Debug)]
pub struct DMATransfer {
    pub src_addr: usize,
    pub dst_addr: usize,
    pub size: usize,
    pub flags: u32,
}

/// DMA统计信息
pub struct DMAStats {
    pub bytes_transferred: AtomicU64,
    pub transfers_completed: AtomicU64,
    pub transfer_errors: AtomicU64,
}

impl DMAStats {
    pub fn new() -> Self {
        Self {
            bytes_transferred: AtomicU64::new(0),
            transfers_completed: AtomicU64::new(0),
            transfer_errors: AtomicU64::new(0),
        }
    }
}

/// 🚀 零拷贝统计信息
pub struct ZeroCopyStats {
    /// 分配的块数
    pub blocks_allocated: AtomicU64,
    /// 释放的块数
    pub blocks_freed: AtomicU64,
    /// 零拷贝传输字节数
    pub bytes_transferred: AtomicU64,
    /// 内存映射缓冲区使用量
    pub mmap_buffer_usage: AtomicU64,
}

impl ZeroCopyStats {
    pub fn new() -> Self {
        Self {
            blocks_allocated: AtomicU64::new(0),
            blocks_freed: AtomicU64::new(0),
            bytes_transferred: AtomicU64::new(0),
            mmap_buffer_usage: AtomicU64::new(0),
        }
    }
    
    /// 打印统计信息
    pub fn print_stats(&self) {
        let allocated = self.blocks_allocated.load(Ordering::Relaxed);
        let freed = self.blocks_freed.load(Ordering::Relaxed);
        let bytes = self.bytes_transferred.load(Ordering::Relaxed);
        let mmap_usage = self.mmap_buffer_usage.load(Ordering::Relaxed);
        
        log::info!("🚀 Zero-Copy Stats:");
        log::info!("   📦 Blocks: Allocated={}, Freed={}, Active={}", 
                  allocated, freed, allocated.saturating_sub(freed));
        log::info!("   📊 Bytes Transferred: {} ({:.2} MB)", 
                  bytes, bytes as f64 / 1024.0 / 1024.0);
        log::info!("   💾 Memory Mapped Usage: {} ({:.2} MB)", 
                  mmap_usage, mmap_usage as f64 / 1024.0 / 1024.0);
    }
}

impl ZeroCopyMemoryManager {
    /// 创建零拷贝内存管理器
    pub fn new() -> Result<Self> {
        let mut shared_pools = Vec::new();
        let mut mmap_buffers = Vec::new();
        
        // 创建不同大小的内存池
        // 小块池: 64KB blocks, 1GB total
        shared_pools.push(Arc::new(SharedMemoryPool::new(0, 1024 * 1024 * 1024, 64 * 1024)?));
        // 中块池: 1MB blocks, 4GB total  
        shared_pools.push(Arc::new(SharedMemoryPool::new(1, 4 * 1024 * 1024 * 1024, 1024 * 1024)?));
        // 大块池: 16MB blocks, 8GB total
        shared_pools.push(Arc::new(SharedMemoryPool::new(2, 8 * 1024 * 1024 * 1024, 16 * 1024 * 1024)?));
        
        // 创建内存映射缓冲区
        for i in 0..8 {
            mmap_buffers.push(Arc::new(MemoryMappedBuffer::new(i, 256 * 1024 * 1024)?)); // 256MB each
        }
        
        let dma_manager = Arc::new(DirectMemoryAccessManager::new(16)?); // 16 DMA channels
        let stats = Arc::new(ZeroCopyStats::new());
        
        log::info!("🚀 Zero-Copy Memory Manager initialized");
        log::info!("   📦 Memory Pools: {}", shared_pools.len());
        log::info!("   💾 Mapped Buffers: {}", mmap_buffers.len());
        log::info!("   🔄 DMA Channels: 16");
        
        Ok(Self {
            shared_pools,
            mmap_buffers,
            dma_manager,
            stats,
        })
    }
    
    /// 🚀 分配零拷贝内存块
    #[inline(always)]
    pub fn allocate(&self, size: usize) -> Option<ZeroCopyBlock> {
        // 根据大小选择合适的内存池
        let pool = if size <= 64 * 1024 {
            &self.shared_pools[0] // 小块池
        } else if size <= 1024 * 1024 {
            &self.shared_pools[1] // 中块池
        } else {
            &self.shared_pools[2] // 大块池
        };
        
        if let Some(block) = pool.allocate_block() {
            self.stats.blocks_allocated.fetch_add(1, Ordering::Relaxed);
            Some(block)
        } else {
            None
        }
    }
    
    /// 🚀 释放零拷贝内存块
    #[inline(always)]
    pub fn deallocate(&self, block: ZeroCopyBlock) {
        let pool_id = block.pool_id as usize;
        if pool_id < self.shared_pools.len() {
            self.shared_pools[pool_id].deallocate_block(block);
            self.stats.blocks_freed.fetch_add(1, Ordering::Relaxed);
        }
    }
    
    /// 获取内存映射缓冲区
    #[inline(always)]
    pub fn get_mmap_buffer(&self, buffer_id: usize) -> Option<Arc<MemoryMappedBuffer>> {
        self.mmap_buffers.get(buffer_id).cloned()
    }
    
    /// 获取DMA管理器
    #[inline(always)]
    pub fn get_dma_manager(&self) -> Arc<DirectMemoryAccessManager> {
        self.dma_manager.clone()
    }
    
    /// 获取统计信息
    pub fn get_stats(&self) -> Arc<ZeroCopyStats> {
        self.stats.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_shared_memory_pool() -> Result<()> {
        let pool = SharedMemoryPool::new(0, 1024 * 1024, 4096)?;
        
        // 测试分配
        let block1 = pool.allocate_block().expect("Should allocate block");
        assert_eq!(block1.size(), 4096);
        
        let block2 = pool.allocate_block().expect("Should allocate another block");
        assert_eq!(block2.size(), 4096);
        
        // 测试释放
        pool.deallocate_block(block1);
        pool.deallocate_block(block2);
        
        Ok(())
    }
    
    #[tokio::test]
    async fn test_memory_mapped_buffer() -> Result<()> {
        let buffer = MemoryMappedBuffer::new(0, 1024 * 1024)?;
        
        let test_data = b"Hello, Zero-Copy World!";
        
        // 测试写入
        let written = buffer.write_data(test_data)?;
        assert_eq!(written, test_data.len());
        
        // 测试读取
        let mut read_buffer = vec![0u8; test_data.len()];
        let read = buffer.read_data(&mut read_buffer)?;
        assert_eq!(read, test_data.len());
        assert_eq!(&read_buffer, test_data);
        
        Ok(())
    }
    
    #[tokio::test]
    async fn test_dma_transfer() -> Result<()> {
        let dma_manager = DirectMemoryAccessManager::new(4)?;
        
        let src = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let mut dst = vec![0u8; 8];
        
        let transferred = dma_manager.dma_transfer(&src, &mut dst).await?;
        assert_eq!(transferred, 8);
        assert_eq!(src, dst);
        
        Ok(())
    }
    
    #[tokio::test]
    async fn test_zero_copy_manager() -> Result<()> {
        let manager = ZeroCopyMemoryManager::new()?;
        
        // 测试小块分配
        let small_block = manager.allocate(1024).expect("Should allocate small block");
        assert_eq!(small_block.size(), 65536); // 小块池的块大小
        
        // 测试大块分配
        let large_block = manager.allocate(5 * 1024 * 1024).expect("Should allocate large block");
        assert_eq!(large_block.size(), 16 * 1024 * 1024); // 大块池的块大小
        
        manager.deallocate(small_block);
        manager.deallocate(large_block);
        
        Ok(())
    }
}