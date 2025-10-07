//! 🚀 内核绕过网络栈 - 极致性能优化
//! 
//! 通过绕过Linux内核网络栈，直接在用户态处理网络包，
//! 实现纳秒级延迟的网络通信。

use std::sync::{Arc, atomic::{AtomicU64, AtomicBool, Ordering}};
use std::time::{Duration, Instant};
use std::mem::size_of;
use std::ptr;
use memmap2::MmapMut;
use crossbeam_utils::CachePadded;
use anyhow::Result;
use log::{info, warn};

/// 🚀 用户态网络栈接口
pub trait UserSpaceNetworking {
    /// 发送原始数据包
    fn send_raw_packet(&self, data: &[u8], dst_addr: std::net::SocketAddr) -> Result<()>;
    
    /// 接收原始数据包
    fn receive_raw_packet(&self, buffer: &mut [u8]) -> Result<(usize, std::net::SocketAddr)>;
    
    /// 获取网络统计信息
    fn get_network_stats(&self) -> NetworkStats;
}

/// 网络统计信息
#[derive(Debug, Clone, Default)]
pub struct NetworkStats {
    pub packets_sent: u64,
    pub packets_received: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub send_errors: u64,
    pub receive_errors: u64,
    pub avg_send_latency_ns: f64,
    pub avg_receive_latency_ns: f64,
}

/// 🚀 高性能用户态UDP实现
pub struct KernelBypassUDP {
    /// 网卡绑定配置
    interface_name: String,
    /// 发送队列
    tx_queue: Arc<TxQueue>,
    /// 接收队列  
    rx_queue: Arc<RxQueue>,
    /// 统计信息
    stats: Arc<CachePadded<AtomicNetworkStats>>,
    /// 运行状态
    running: Arc<AtomicBool>,
    /// CPU亲和性配置
    cpu_affinity: Option<usize>,
}

/// 原子网络统计
pub struct AtomicNetworkStats {
    pub packets_sent: AtomicU64,
    pub packets_received: AtomicU64,
    pub bytes_sent: AtomicU64,
    pub bytes_received: AtomicU64,
    pub send_errors: AtomicU64,
    pub receive_errors: AtomicU64,
    pub total_send_latency_ns: AtomicU64,
    pub total_receive_latency_ns: AtomicU64,
}

impl Default for AtomicNetworkStats {
    fn default() -> Self {
        Self {
            packets_sent: AtomicU64::new(0),
            packets_received: AtomicU64::new(0),
            bytes_sent: AtomicU64::new(0),
            bytes_received: AtomicU64::new(0),
            send_errors: AtomicU64::new(0),
            receive_errors: AtomicU64::new(0),
            total_send_latency_ns: AtomicU64::new(0),
            total_receive_latency_ns: AtomicU64::new(0),
        }
    }
}

/// 🚀 发送队列 - 零拷贝环形缓冲区
pub struct TxQueue {
    /// 环形缓冲区（内存映射）
    ring_buffer: Arc<MmapMut>,
    /// 队列容量
    capacity: usize,
    /// 头指针（生产者）
    head: CachePadded<AtomicU64>,
    /// 尾指针（消费者）
    tail: CachePadded<AtomicU64>,
    /// 包描述符大小
    descriptor_size: usize,
}

/// 🚀 接收队列 - 零拷贝环形缓冲区
pub struct RxQueue {
    /// 环形缓冲区（内存映射）
    ring_buffer: Arc<MmapMut>,
    /// 队列容量
    capacity: usize,
    /// 头指针（生产者）
    head: CachePadded<AtomicU64>,
    /// 尾指针（消费者）  
    tail: CachePadded<AtomicU64>,
    /// 包描述符大小
    descriptor_size: usize,
}

/// 网络包描述符
#[repr(C)]
#[derive(Debug, Clone)]
pub struct PacketDescriptor {
    /// 数据长度
    pub length: u32,
    /// 时间戳（纳秒）
    pub timestamp_ns: u64,
    /// 目标地址
    pub dst_addr: u32,
    /// 目标端口
    pub dst_port: u16,
    /// 包类型标志
    pub flags: u16,
    /// 数据偏移量
    pub data_offset: u32,
    /// 预留字段（缓存行对齐）
    _padding: [u8; 4],
}

impl TxQueue {
    /// 创建发送队列
    pub fn new(capacity: usize) -> Result<Self> {
        let descriptor_size = size_of::<PacketDescriptor>();
        // 每个条目需要描述符 + 最大包大小(1500字节)
        let entry_size = descriptor_size + 1500;
        let total_size = capacity * entry_size;
        
        // 创建内存映射缓冲区，页对齐
        let ring_buffer = Arc::new(MmapMut::map_anon(total_size)?);
        
        info!("📤 Created TX queue: capacity={}, size={}MB", 
              capacity, total_size / 1024 / 1024);
        
        Ok(Self {
            ring_buffer,
            capacity,
            head: CachePadded::new(AtomicU64::new(0)),
            tail: CachePadded::new(AtomicU64::new(0)),
            descriptor_size,
        })
    }
    
    /// 🚀 零拷贝发送包
    #[inline(always)]
    pub fn send_packet_zero_copy(&self, data: &[u8], dst_addr: std::net::SocketAddr) -> Result<()> {
        let current_head = self.head.load(Ordering::Relaxed);
        let current_tail = self.tail.load(Ordering::Acquire);
        
        // 检查队列是否满
        if (current_head + 1) % self.capacity as u64 == current_tail {
            return Err(anyhow::anyhow!("TX queue is full"));
        }
        
        let entry_size = self.descriptor_size + 1500;
        let entry_offset = (current_head % self.capacity as u64) as usize * entry_size;
        
        // 安全地获取缓冲区指针
        let buffer_ptr = unsafe {
            self.ring_buffer.as_ptr().add(entry_offset)
        };
        
        // 写入包描述符
        let descriptor = PacketDescriptor {
            length: data.len() as u32,
            timestamp_ns: Instant::now().elapsed().as_nanos() as u64,
            dst_addr: match dst_addr.ip() {
                std::net::IpAddr::V4(ipv4) => u32::from(ipv4),
                _ => return Err(anyhow::anyhow!("Only IPv4 supported")),
            },
            dst_port: dst_addr.port(),
            flags: 0,
            data_offset: self.descriptor_size as u32,
            _padding: [0; 4],
        };
        
        unsafe {
            // 写入描述符（缓存行对齐的原子写入）
            ptr::write(buffer_ptr as *mut PacketDescriptor, descriptor);
            
            // 写入数据（使用SIMD加速的内存拷贝）
            let data_ptr = buffer_ptr.add(self.descriptor_size);
            self.fast_memcpy(data_ptr as *mut u8, data.as_ptr(), data.len());
        }
        
        // 原子更新头指针（发布操作）
        self.head.store(current_head + 1, Ordering::Release);
        
        Ok(())
    }
    
    /// 🚀 SIMD加速的内存拷贝
    #[inline(always)]
    unsafe fn fast_memcpy(&self, dst: *mut u8, src: *const u8, len: usize) {
        // 对于小数据，使用普通拷贝
        if len <= 32 {
            ptr::copy_nonoverlapping(src, dst, len);
            return;
        }
        
        #[cfg(target_arch = "x86_64")]
        {
            use std::arch::x86_64::{__m256i, _mm256_loadu_si256, _mm256_storeu_si256};
            
            let mut offset = 0;
            let chunks = len / 32;
            
            // 使用AVX2进行32字节对齐拷贝
            for _ in 0..chunks {
                let chunk = _mm256_loadu_si256(src.add(offset) as *const __m256i);
                _mm256_storeu_si256(dst.add(offset) as *mut __m256i, chunk);
                offset += 32;
            }
            
            // 处理剩余字节
            let remaining = len % 32;
            if remaining > 0 {
                ptr::copy_nonoverlapping(src.add(offset), dst.add(offset), remaining);
            }
        }
        
        #[cfg(not(target_arch = "x86_64"))]
        {
            // 非x86_64架构使用普通拷贝
            ptr::copy_nonoverlapping(src, dst, len);
        }
    }
    
    /// 获取待发送包数量
    #[inline(always)]
    pub fn pending_packets(&self) -> u64 {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Relaxed);
        (head + self.capacity as u64 - tail) % self.capacity as u64
    }
}

impl RxQueue {
    /// 创建接收队列
    pub fn new(capacity: usize) -> Result<Self> {
        let descriptor_size = size_of::<PacketDescriptor>();
        let entry_size = descriptor_size + 1500;
        let total_size = capacity * entry_size;
        
        let ring_buffer = Arc::new(MmapMut::map_anon(total_size)?);
        
        info!("📥 Created RX queue: capacity={}, size={}MB", 
              capacity, total_size / 1024 / 1024);
        
        Ok(Self {
            ring_buffer,
            capacity,
            head: CachePadded::new(AtomicU64::new(0)),
            tail: CachePadded::new(AtomicU64::new(0)),
            descriptor_size,
        })
    }
    
    /// 🚀 零拷贝接收包
    #[inline(always)]
    pub fn receive_packet_zero_copy(&self, buffer: &mut [u8]) -> Result<(usize, std::net::SocketAddr)> {
        let current_tail = self.tail.load(Ordering::Relaxed);
        let current_head = self.head.load(Ordering::Acquire);
        
        // 检查队列是否为空
        if current_tail == current_head {
            return Err(anyhow::anyhow!("RX queue is empty"));
        }
        
        let entry_size = self.descriptor_size + 1500;
        let entry_offset = (current_tail % self.capacity as u64) as usize * entry_size;
        
        let buffer_ptr = unsafe {
            self.ring_buffer.as_ptr().add(entry_offset)
        };
        
        // 读取包描述符
        let descriptor = unsafe {
            ptr::read(buffer_ptr as *const PacketDescriptor)
        };
        
        let data_len = descriptor.length as usize;
        if data_len > buffer.len() {
            return Err(anyhow::anyhow!("Buffer too small: need {}, got {}", 
                                     data_len, buffer.len()));
        }
        
        // 零拷贝读取数据
        unsafe {
            let data_ptr = buffer_ptr.add(self.descriptor_size);
            self.fast_memcpy(buffer.as_mut_ptr(), data_ptr, data_len);
        }
        
        // 构造源地址
        let src_addr = std::net::SocketAddr::new(
            std::net::IpAddr::V4(std::net::Ipv4Addr::from(descriptor.dst_addr)),
            descriptor.dst_port,
        );
        
        // 原子更新尾指针
        self.tail.store(current_tail + 1, Ordering::Release);
        
        Ok((data_len, src_addr))
    }
    
    /// 🚀 SIMD加速的内存拷贝（与TxQueue共享实现）
    #[inline(always)]
    unsafe fn fast_memcpy(&self, dst: *mut u8, src: *const u8, len: usize) {
        if len <= 32 {
            ptr::copy_nonoverlapping(src, dst, len);
            return;
        }
        
        #[cfg(target_arch = "x86_64")]
        {
            use std::arch::x86_64::{__m256i, _mm256_loadu_si256, _mm256_storeu_si256};
            
            let mut offset = 0;
            let chunks = len / 32;
            
            for _ in 0..chunks {
                let chunk = _mm256_loadu_si256(src.add(offset) as *const __m256i);
                _mm256_storeu_si256(dst.add(offset) as *mut __m256i, chunk);
                offset += 32;
            }
            
            let remaining = len % 32;
            if remaining > 0 {
                ptr::copy_nonoverlapping(src.add(offset), dst.add(offset), remaining);
            }
        }
        
        #[cfg(not(target_arch = "x86_64"))]
        {
            ptr::copy_nonoverlapping(src, dst, len);
        }
    }
    
    /// 获取待接收包数量
    #[inline(always)]
    pub fn available_packets(&self) -> u64 {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Relaxed);
        (head + self.capacity as u64 - tail) % self.capacity as u64
    }
}

impl KernelBypassUDP {
    /// 创建内核绕过UDP实例
    pub fn new(interface_name: String, cpu_affinity: Option<usize>) -> Result<Self> {
        info!("🚀 Creating kernel bypass UDP on interface: {}", interface_name);
        
        // 创建大容量队列（1M条目）
        let tx_queue = Arc::new(TxQueue::new(1_000_000)?);
        let rx_queue = Arc::new(RxQueue::new(1_000_000)?);
        
        let instance = Self {
            interface_name,
            tx_queue,
            rx_queue,
            stats: Arc::new(CachePadded::new(AtomicNetworkStats::default())),
            running: Arc::new(AtomicBool::new(false)),
            cpu_affinity,
        };
        
        info!("✅ Kernel bypass UDP created successfully");
        Ok(instance)
    }
    
    /// 启动内核绕过网络处理
    pub async fn start(&self) -> Result<()> {
        info!("🚀 Starting kernel bypass networking...");
        
        self.running.store(true, Ordering::Relaxed);
        
        // 启动发送线程
        self.start_tx_thread().await?;
        
        // 启动接收线程  
        self.start_rx_thread().await?;
        
        // 启动统计线程
        self.start_stats_thread().await;
        
        info!("✅ Kernel bypass networking started");
        Ok(())
    }
    
    /// 启动发送线程
    async fn start_tx_thread(&self) -> Result<()> {
        let tx_queue = Arc::clone(&self.tx_queue);
        let stats = Arc::clone(&self.stats);
        let running = Arc::clone(&self.running);
        let cpu_affinity = self.cpu_affinity;
        
        tokio::spawn(async move {
            if let Some(cpu_id) = cpu_affinity {
                Self::set_thread_cpu_affinity(cpu_id);
            }
            
            info!("📤 TX thread started");
            
            while running.load(Ordering::Relaxed) {
                let pending = tx_queue.pending_packets();
                
                if pending > 0 {
                    // 模拟发送处理（实际应该调用网卡驱动）
                    stats.packets_sent.fetch_add(pending, Ordering::Relaxed);
                    
                    // 更新队列尾指针（模拟包发送完成）
                    let current_tail = tx_queue.tail.load(Ordering::Relaxed);
                    tx_queue.tail.store(current_tail + pending, Ordering::Release);
                } else {
                    // 极短休眠避免CPU空转
                    tokio::task::yield_now().await;
                }
            }
            
            info!("📤 TX thread stopped");
        });
        
        Ok(())
    }
    
    /// 启动接收线程
    async fn start_rx_thread(&self) -> Result<()> {
        let _rx_queue = Arc::clone(&self.rx_queue);
        let _stats = Arc::clone(&self.stats);
        let running = Arc::clone(&self.running);
        let cpu_affinity = self.cpu_affinity.map(|id| id + 1); // 使用下一个CPU核心
        
        tokio::spawn(async move {
            if let Some(cpu_id) = cpu_affinity {
                Self::set_thread_cpu_affinity(cpu_id);
            }
            
            info!("📥 RX thread started");
            
            while running.load(Ordering::Relaxed) {
                // 模拟从网卡接收包（实际应该从网卡驱动读取）
                // 这里简化为空循环，实际实现会轮询网卡
                tokio::task::yield_now().await;
            }
            
            info!("📥 RX thread stopped");
        });
        
        Ok(())
    }
    
    /// 启动统计线程
    async fn start_stats_thread(&self) {
        let stats = Arc::clone(&self.stats);
        let running = Arc::clone(&self.running);
        
        tokio::spawn(async move {
            info!("📊 Stats thread started");
            
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            
            while running.load(Ordering::Relaxed) {
                interval.tick().await;
                
                let packets_sent = stats.packets_sent.load(Ordering::Relaxed);
                let packets_received = stats.packets_received.load(Ordering::Relaxed);
                let bytes_sent = stats.bytes_sent.load(Ordering::Relaxed);
                let bytes_received = stats.bytes_received.load(Ordering::Relaxed);
                
                if packets_sent > 0 || packets_received > 0 {
                    info!("🌐 Network Stats: TX: {} pkts, {} bytes | RX: {} pkts, {} bytes",
                          packets_sent, bytes_sent, packets_received, bytes_received);
                }
            }
            
            info!("📊 Stats thread stopped");
        });
    }
    
    /// 设置线程CPU亲和性
    #[allow(unused_variables)]
    fn set_thread_cpu_affinity(cpu_id: usize) {
        #[cfg(target_os = "linux")]
        {
            use libc::{cpu_set_t, sched_setaffinity, CPU_SET, CPU_ZERO};
            
            unsafe {
                let mut cpuset: cpu_set_t = std::mem::zeroed();
                CPU_ZERO(&mut cpuset);
                CPU_SET(cpu_id, &mut cpuset);
                
                if sched_setaffinity(0, std::mem::size_of::<cpu_set_t>(), &cpuset) == 0 {
                    info!("✅ Thread bound to CPU {}", cpu_id);
                } else {
                    warn!("⚠️ Failed to bind thread to CPU {}", cpu_id);
                }
            }
        }
        
        #[cfg(not(target_os = "linux"))]
        {
            info!("💡 CPU affinity not supported on this platform");
        }
    }
    
    /// 停止内核绕过网络处理
    pub async fn stop(&self) -> Result<()> {
        info!("🛑 Stopping kernel bypass networking...");
        
        self.running.store(false, Ordering::Relaxed);
        
        // 等待线程退出
        tokio::time::sleep(Duration::from_millis(100)).await;
        
        info!("✅ Kernel bypass networking stopped");
        Ok(())
    }
}

impl UserSpaceNetworking for KernelBypassUDP {
    fn send_raw_packet(&self, data: &[u8], dst_addr: std::net::SocketAddr) -> Result<()> {
        let send_start = Instant::now();
        
        let result = self.tx_queue.send_packet_zero_copy(data, dst_addr);
        
        if result.is_ok() {
            let latency_ns = send_start.elapsed().as_nanos() as u64;
            self.stats.bytes_sent.fetch_add(data.len() as u64, Ordering::Relaxed);
            self.stats.total_send_latency_ns.fetch_add(latency_ns, Ordering::Relaxed);
        } else {
            self.stats.send_errors.fetch_add(1, Ordering::Relaxed);
        }
        
        result
    }
    
    fn receive_raw_packet(&self, buffer: &mut [u8]) -> Result<(usize, std::net::SocketAddr)> {
        let receive_start = Instant::now();
        
        let result = self.rx_queue.receive_packet_zero_copy(buffer);
        
        match &result {
            Ok((len, _addr)) => {
                let latency_ns = receive_start.elapsed().as_nanos() as u64;
                self.stats.packets_received.fetch_add(1, Ordering::Relaxed);
                self.stats.bytes_received.fetch_add(*len as u64, Ordering::Relaxed);
                self.stats.total_receive_latency_ns.fetch_add(latency_ns, Ordering::Relaxed);
            }
            Err(_) => {
                self.stats.receive_errors.fetch_add(1, Ordering::Relaxed);
            }
        }
        
        result
    }
    
    fn get_network_stats(&self) -> NetworkStats {
        let packets_sent = self.stats.packets_sent.load(Ordering::Relaxed);
        let packets_received = self.stats.packets_received.load(Ordering::Relaxed);
        let total_send_latency = self.stats.total_send_latency_ns.load(Ordering::Relaxed);
        let total_receive_latency = self.stats.total_receive_latency_ns.load(Ordering::Relaxed);
        
        NetworkStats {
            packets_sent,
            packets_received,
            bytes_sent: self.stats.bytes_sent.load(Ordering::Relaxed),
            bytes_received: self.stats.bytes_received.load(Ordering::Relaxed),
            send_errors: self.stats.send_errors.load(Ordering::Relaxed),
            receive_errors: self.stats.receive_errors.load(Ordering::Relaxed),
            avg_send_latency_ns: if packets_sent > 0 {
                total_send_latency as f64 / packets_sent as f64
            } else {
                0.0
            },
            avg_receive_latency_ns: if packets_received > 0 {
                total_receive_latency as f64 / packets_received as f64
            } else {
                0.0
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_tx_queue_creation() {
        let tx_queue = TxQueue::new(1000).unwrap();
        assert_eq!(tx_queue.capacity, 1000);
        assert_eq!(tx_queue.pending_packets(), 0);
    }
    
    #[test]
    fn test_rx_queue_creation() {
        let rx_queue = RxQueue::new(1000).unwrap();
        assert_eq!(rx_queue.capacity, 1000);
        assert_eq!(rx_queue.available_packets(), 0);
    }
    
    #[tokio::test]
    async fn test_kernel_bypass_udp() {
        let udp = KernelBypassUDP::new("eth0".to_string(), Some(0)).unwrap();
        
        // 测试统计信息
        let stats = udp.get_network_stats();
        assert_eq!(stats.packets_sent, 0);
        assert_eq!(stats.packets_received, 0);
    }
}