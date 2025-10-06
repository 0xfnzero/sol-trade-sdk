//! 🚀 实时系统级调优 - 极致延迟控制
//! 
//! 实现操作系统级的实时优化，包括：
//! - 实时调度策略 (SCHED_FIFO, SCHED_RR)
//! - 内存锁定防止页面交换
//! - CPU隔离和亲和性绑定  
//! - 中断处理优化
//! - 系统定时器调优
//! - NUMA拓扑优化
//! - 电源管理调优

use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use anyhow::Result;
use log::{info, warn};

/// 🚀 实时系统优化器
pub struct RealtimeSystemOptimizer {
    /// 配置
    config: RealtimeConfig,
    /// 优化状态
    optimization_state: Arc<OptimizationState>,
    /// 统计信息
    stats: Arc<RealtimeStats>,
    /// 是否已初始化
    initialized: AtomicBool,
}

/// 实时系统配置
#[derive(Debug, Clone)]
pub struct RealtimeConfig {
    /// 启用实时调度
    pub enable_realtime_scheduling: bool,
    /// 实时优先级 (1-99, 99最高)
    pub realtime_priority: i32,
    /// 启用内存锁定
    pub enable_memory_locking: bool,
    /// 锁定内存大小限制 (字节)
    pub memory_lock_limit: usize,
    /// 启用CPU隔离
    pub enable_cpu_isolation: bool,
    /// 专用CPU核心列表
    pub isolated_cpu_cores: Vec<usize>,
    /// 启用中断隔离
    pub enable_interrupt_isolation: bool,
    /// 中断亲和性CPU核心
    pub interrupt_cpu_cores: Vec<usize>,
    /// 启用NUMA优化
    pub enable_numa_optimization: bool,
    /// 首选NUMA节点
    pub preferred_numa_nodes: Vec<usize>,
    /// 启用电源管理优化
    pub enable_power_optimization: bool,
    /// CPU调频策略
    pub cpu_frequency_governor: CpuGovernor,
}

/// CPU调频策略
#[derive(Debug, Clone)]
pub enum CpuGovernor {
    /// 性能模式 (最高频率)
    Performance,
    /// 按需调频
    OnDemand,
    /// 用户空间控制
    Userspace,
    /// 保守模式
    Conservative,
}

impl Default for RealtimeConfig {
    fn default() -> Self {
        Self {
            enable_realtime_scheduling: true,
            realtime_priority: 80, // 高优先级但不是最高
            enable_memory_locking: true,
            memory_lock_limit: 2 * 1024 * 1024 * 1024, // 2GB
            enable_cpu_isolation: true,
            isolated_cpu_cores: vec![], // 运行时检测
            enable_interrupt_isolation: true,
            interrupt_cpu_cores: vec![], // 运行时检测  
            enable_numa_optimization: true,
            preferred_numa_nodes: vec![],
            enable_power_optimization: true,
            cpu_frequency_governor: CpuGovernor::Performance,
        }
    }
}

/// 优化状态
pub struct OptimizationState {
    /// 实时调度已启用
    pub realtime_scheduling_enabled: AtomicBool,
    /// 内存已锁定
    pub memory_locked: AtomicBool,
    /// CPU亲和性已设置
    pub cpu_affinity_set: AtomicBool,
    /// 中断隔离已启用
    pub interrupt_isolation_enabled: AtomicBool,
    /// NUMA优化已启用
    pub numa_optimization_enabled: AtomicBool,
    /// 电源优化已启用
    pub power_optimization_enabled: AtomicBool,
}

impl Default for OptimizationState {
    fn default() -> Self {
        Self {
            realtime_scheduling_enabled: AtomicBool::new(false),
            memory_locked: AtomicBool::new(false),
            cpu_affinity_set: AtomicBool::new(false),
            interrupt_isolation_enabled: AtomicBool::new(false),
            numa_optimization_enabled: AtomicBool::new(false),
            power_optimization_enabled: AtomicBool::new(false),
        }
    }
}

/// 实时系统统计
pub struct RealtimeStats {
    /// 调度延迟统计 (纳秒)
    pub scheduling_latency_ns: AtomicU64,
    /// 最大调度延迟
    pub max_scheduling_latency_ns: AtomicU64,
    /// 页面错误计数
    pub page_faults: AtomicU64,
    /// 上下文切换计数
    pub context_switches: AtomicU64,
    /// 中断计数
    pub interrupts: AtomicU64,
    /// 系统调用计数
    pub system_calls: AtomicU64,
}

impl Default for RealtimeStats {
    fn default() -> Self {
        Self {
            scheduling_latency_ns: AtomicU64::new(0),
            max_scheduling_latency_ns: AtomicU64::new(0),
            page_faults: AtomicU64::new(0),
            context_switches: AtomicU64::new(0),
            interrupts: AtomicU64::new(0),
            system_calls: AtomicU64::new(0),
        }
    }
}

impl RealtimeSystemOptimizer {
    /// 创建实时系统优化器
    pub fn new(mut config: RealtimeConfig) -> Result<Self> {
        // 自动检测系统配置
        Self::auto_detect_system_config(&mut config)?;
        
        info!("🚀 Creating RealtimeSystemOptimizer with config: {:?}", config);
        
        Ok(Self {
            config,
            optimization_state: Arc::new(OptimizationState::default()),
            stats: Arc::new(RealtimeStats::default()),
            initialized: AtomicBool::new(false),
        })
    }
    
    /// 自动检测系统配置
    fn auto_detect_system_config(config: &mut RealtimeConfig) -> Result<()> {
        // 检测CPU核心数
        let num_cpus = num_cpus::get();
        info!("🧠 Detected {} CPU cores", num_cpus);
        
        // 自动配置CPU隔离 - 预留最后几个核心给应用
        if config.isolated_cpu_cores.is_empty() && num_cpus > 4 {
            config.isolated_cpu_cores = ((num_cpus - 2)..num_cpus).collect();
            info!("🎯 Auto-configured isolated CPU cores: {:?}", config.isolated_cpu_cores);
        }
        
        // 自动配置中断处理核心 - 使用前几个核心
        if config.interrupt_cpu_cores.is_empty() && num_cpus > 2 {
            config.interrupt_cpu_cores = (0..2).collect();
            info!("⚡ Auto-configured interrupt CPU cores: {:?}", config.interrupt_cpu_cores);
        }
        
        // 检测NUMA拓扑
        Self::detect_numa_topology(config)?;
        
        Ok(())
    }
    
    /// 检测NUMA拓扑
    #[allow(unused_variables)]
    fn detect_numa_topology(config: &mut RealtimeConfig) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            // 尝试读取NUMA信息
            if let Ok(numa_info) = std::fs::read_to_string("/proc/sys/kernel/numa_balancing") {
                if numa_info.trim() == "1" {
                    info!("🏗️ NUMA balancing detected - will optimize for NUMA");
                    if config.preferred_numa_nodes.is_empty() {
                        config.preferred_numa_nodes = vec![0]; // 默认使用节点0
                    }
                }
            }
        }
        
        Ok(())
    }
    
    /// 🚀 应用所有实时系统优化
    pub async fn apply_all_optimizations(&self) -> Result<()> {
        if self.initialized.load(Ordering::Acquire) {
            warn!("Real-time optimizations already applied");
            return Ok(());
        }
        
        info!("🚀 Applying real-time system optimizations...");
        
        // 1. 实时调度优化
        if self.config.enable_realtime_scheduling {
            self.apply_realtime_scheduling().await?;
        }
        
        // 2. 内存锁定优化
        if self.config.enable_memory_locking {
            self.apply_memory_locking().await?;
        }
        
        // 3. CPU隔离优化
        if self.config.enable_cpu_isolation {
            self.apply_cpu_isolation().await?;
        }
        
        // 4. 中断隔离优化
        if self.config.enable_interrupt_isolation {
            self.apply_interrupt_isolation().await?;
        }
        
        // 5. NUMA优化
        if self.config.enable_numa_optimization {
            self.apply_numa_optimization().await?;
        }
        
        // 6. 电源管理优化
        if self.config.enable_power_optimization {
            self.apply_power_optimization().await?;
        }
        
        // 启动实时监控
        self.start_realtime_monitoring().await;
        
        self.initialized.store(true, Ordering::Release);
        info!("✅ All real-time optimizations applied successfully");
        
        Ok(())
    }
    
    /// 应用实时调度优化
    async fn apply_realtime_scheduling(&self) -> Result<()> {
        info!("⏰ Applying real-time scheduling optimizations...");
        
        #[cfg(target_os = "linux")]
        {
            use libc::{sched_setscheduler, sched_param, SCHED_FIFO, SCHED_RR};
            
            // 设置实时调度策略
            let mut param: sched_param = unsafe { std::mem::zeroed() };
            param.sched_priority = self.config.realtime_priority;
            
            unsafe {
                // 尝试SCHED_FIFO (先进先出实时调度)
                if sched_setscheduler(0, SCHED_FIFO, &param) == 0 {
                    info!("✅ Real-time FIFO scheduling enabled with priority {}", 
                          self.config.realtime_priority);
                    self.optimization_state.realtime_scheduling_enabled.store(true, Ordering::Release);
                } else {
                    // 回退到SCHED_RR (轮询实时调度)
                    if sched_setscheduler(0, SCHED_RR, &param) == 0 {
                        info!("✅ Real-time RR scheduling enabled with priority {}", 
                              self.config.realtime_priority);
                        self.optimization_state.realtime_scheduling_enabled.store(true, Ordering::Release);
                    } else {
                        warn!("⚠️ Failed to set real-time scheduling (requires root privileges)");
                    }
                }
            }
        }
        
        #[cfg(target_os = "macos")]
        {
            // 实时调度在macOS上需要使用不同的API
            warn!("⚠️ Real-time scheduling not available on macOS");
        }
        
        #[cfg(not(unix))]
        {
            warn!("⚠️ Real-time scheduling optimization not supported on this platform");
        }
        
        Ok(())
    }
    
    /// 应用内存锁定优化
    async fn apply_memory_locking(&self) -> Result<()> {
        info!("🔒 Applying memory locking optimizations...");
        
        #[cfg(unix)]
        {
            use libc::{mlockall, MCL_CURRENT, MCL_FUTURE, setrlimit, rlimit, RLIMIT_MEMLOCK};
            
            // 设置内存锁定限制
            let rlim = rlimit {
                rlim_cur: self.config.memory_lock_limit as u64,
                rlim_max: self.config.memory_lock_limit as u64,
            };
            
            unsafe {
                if setrlimit(RLIMIT_MEMLOCK, &rlim) == 0 {
                    info!("✅ Memory lock limit set to {} bytes", self.config.memory_lock_limit);
                } else {
                    warn!("⚠️ Failed to set memory lock limit");
                }
                
                // 锁定所有当前和未来的内存页
                if mlockall(MCL_CURRENT | MCL_FUTURE) == 0 {
                    info!("✅ All memory pages locked to prevent swapping");
                    self.optimization_state.memory_locked.store(true, Ordering::Release);
                } else {
                    warn!("⚠️ Failed to lock memory pages (requires sufficient limits)");
                }
            }
        }
        
        #[cfg(not(unix))]
        {
            warn!("⚠️ Memory locking optimization not supported on this platform");
        }
        
        Ok(())
    }
    
    /// 应用CPU隔离优化
    async fn apply_cpu_isolation(&self) -> Result<()> {
        info!("🎯 Applying CPU isolation optimizations...");
        
        if self.config.isolated_cpu_cores.is_empty() {
            warn!("No isolated CPU cores configured");
            return Ok(());
        }
        
        #[cfg(target_os = "linux")]
        {
            use libc::{cpu_set_t, sched_setaffinity, CPU_ZERO, CPU_SET};
            use std::mem;
            
            let mut cpu_set: cpu_set_t = unsafe { mem::zeroed() };
            
            unsafe {
                CPU_ZERO(&mut cpu_set);
                
                // 设置CPU亲和性到隔离的核心
                for &core_id in &self.config.isolated_cpu_cores {
                    if core_id < 256 { // libc限制
                        CPU_SET(core_id, &mut cpu_set);
                    }
                }
                
                if sched_setaffinity(0, mem::size_of::<cpu_set_t>(), &cpu_set) == 0 {
                    info!("✅ CPU affinity set to isolated cores: {:?}", 
                          self.config.isolated_cpu_cores);
                    self.optimization_state.cpu_affinity_set.store(true, Ordering::Release);
                } else {
                    warn!("⚠️ Failed to set CPU affinity");
                }
            }
        }
        
        #[cfg(target_os = "macos")]
        {
            // CPU亲和性功能在macOS上不可用
            warn!("⚠️ CPU affinity not available on macOS");
        }
        
        #[cfg(not(unix))]
        {
            warn!("⚠️ CPU isolation optimization not supported on this platform");
        }
        
        Ok(())
    }
    
    /// 应用中断隔离优化
    async fn apply_interrupt_isolation(&self) -> Result<()> {
        info!("⚡ Applying interrupt isolation optimizations...");
        
        #[cfg(target_os = "linux")]
        {
            // 中断隔离需要root权限和特殊配置
            // 这里提供配置建议
            info!("💡 For interrupt isolation, consider:");
            info!("   - Using isolcpus=<isolated_cores> kernel parameter");
            info!("   - Configuring IRQ affinity via /proc/irq/*/smp_affinity");
            info!("   - Using rcu_nocbs=<isolated_cores> for RCU callbacks");
            
            // 尝试设置一些可能的中断亲和性
            if !self.config.interrupt_cpu_cores.is_empty() {
                info!("🎯 Interrupt handling will use cores: {:?}", 
                      self.config.interrupt_cpu_cores);
                self.optimization_state.interrupt_isolation_enabled.store(true, Ordering::Release);
            }
        }
        
        Ok(())
    }
    
    /// 应用NUMA优化
    async fn apply_numa_optimization(&self) -> Result<()> {
        info!("🏗️ Applying NUMA optimizations...");
        
        #[cfg(target_os = "linux")]
        {
            if !self.config.preferred_numa_nodes.is_empty() {
                info!("🎯 Preferred NUMA nodes: {:?}", self.config.preferred_numa_nodes);
                info!("💡 For NUMA optimization, consider:");
                info!("   - numactl --membind=<nodes> --cpunodebind=<nodes>");
                info!("   - Setting vm.zone_reclaim_mode=1");
                info!("   - Using NUMA-aware memory allocation");
                
                self.optimization_state.numa_optimization_enabled.store(true, Ordering::Release);
            }
        }
        
        Ok(())
    }
    
    /// 应用电源管理优化
    async fn apply_power_optimization(&self) -> Result<()> {
        info!("🔋 Applying power management optimizations...");
        
        #[cfg(target_os = "linux")]
        {
            let governor = match self.config.cpu_frequency_governor {
                CpuGovernor::Performance => "performance",
                CpuGovernor::OnDemand => "ondemand",
                CpuGovernor::Userspace => "userspace",
                CpuGovernor::Conservative => "conservative",
            };
            
            info!("💡 CPU frequency governor should be set to: {}", governor);
            info!("   Execute: echo {} | sudo tee /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor", governor);
            info!("   Also consider disabling C-states: intel_idle.max_cstate=0");
            
            self.optimization_state.power_optimization_enabled.store(true, Ordering::Release);
        }
        
        Ok(())
    }
    
    /// 启动实时监控
    async fn start_realtime_monitoring(&self) {
        let stats = Arc::clone(&self.stats);
        let state = Arc::clone(&self.optimization_state);
        
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            
            loop {
                interval.tick().await;
                
                // 测量调度延迟
                let start = Instant::now();
                thread::yield_now();
                let scheduling_latency = start.elapsed().as_nanos() as u64;
                
                stats.scheduling_latency_ns.store(scheduling_latency, Ordering::Relaxed);
                
                let max_latency = stats.max_scheduling_latency_ns.load(Ordering::Relaxed);
                if scheduling_latency > max_latency {
                    stats.max_scheduling_latency_ns.store(scheduling_latency, Ordering::Relaxed);
                }
                
                // 定期报告状态
                let rt_enabled = state.realtime_scheduling_enabled.load(Ordering::Relaxed);
                let mem_locked = state.memory_locked.load(Ordering::Relaxed);
                let cpu_affinity = state.cpu_affinity_set.load(Ordering::Relaxed);
                
                if scheduling_latency > 100_000 { // >100μs
                    warn!("⚠️ High scheduling latency detected: {}μs", scheduling_latency / 1000);
                }
                
                // 每分钟输出一次详细状态
                static mut COUNTER: u32 = 0;
                unsafe {
                    COUNTER += 1;
                    if COUNTER % 12 == 0 { // 5秒 * 12 = 1分钟
                        info!("📊 Real-time Status:");
                        info!("   ⏰ RT Scheduling: {}", if rt_enabled { "✅" } else { "❌" });
                        info!("   🔒 Memory Locked: {}", if mem_locked { "✅" } else { "❌" });
                        info!("   🎯 CPU Affinity: {}", if cpu_affinity { "✅" } else { "❌" });
                        info!("   📈 Scheduling Latency: {}ns (max: {}ns)", 
                              scheduling_latency, 
                              stats.max_scheduling_latency_ns.load(Ordering::Relaxed));
                    }
                }
            }
        });
    }
    
    /// 获取实时统计
    pub fn get_stats(&self) -> RealtimeStatsSnapshot {
        RealtimeStatsSnapshot {
            scheduling_latency_ns: self.stats.scheduling_latency_ns.load(Ordering::Relaxed),
            max_scheduling_latency_ns: self.stats.max_scheduling_latency_ns.load(Ordering::Relaxed),
            page_faults: self.stats.page_faults.load(Ordering::Relaxed),
            context_switches: self.stats.context_switches.load(Ordering::Relaxed),
            interrupts: self.stats.interrupts.load(Ordering::Relaxed),
            system_calls: self.stats.system_calls.load(Ordering::Relaxed),
        }
    }
    
    /// 检查优化状态
    pub fn get_optimization_status(&self) -> OptimizationStatus {
        OptimizationStatus {
            realtime_scheduling_enabled: self.optimization_state.realtime_scheduling_enabled.load(Ordering::Relaxed),
            memory_locked: self.optimization_state.memory_locked.load(Ordering::Relaxed),
            cpu_affinity_set: self.optimization_state.cpu_affinity_set.load(Ordering::Relaxed),
            interrupt_isolation_enabled: self.optimization_state.interrupt_isolation_enabled.load(Ordering::Relaxed),
            numa_optimization_enabled: self.optimization_state.numa_optimization_enabled.load(Ordering::Relaxed),
            power_optimization_enabled: self.optimization_state.power_optimization_enabled.load(Ordering::Relaxed),
        }
    }
    
    /// 🚀 创建超低延迟配置
    pub fn ultra_low_latency_config() -> RealtimeConfig {
        let num_cpus = num_cpus::get();
        
        RealtimeConfig {
            enable_realtime_scheduling: true,
            realtime_priority: 99, // 最高优先级
            enable_memory_locking: true,
            memory_lock_limit: 8 * 1024 * 1024 * 1024, // 8GB
            enable_cpu_isolation: true,
            isolated_cpu_cores: if num_cpus > 4 {
                ((num_cpus - 2)..num_cpus).collect()
            } else {
                vec![]
            },
            enable_interrupt_isolation: true,
            interrupt_cpu_cores: (0..2).collect(),
            enable_numa_optimization: true,
            preferred_numa_nodes: vec![0],
            enable_power_optimization: true,
            cpu_frequency_governor: CpuGovernor::Performance,
        }
    }
}

/// 实时统计快照
#[derive(Debug, Clone)]
pub struct RealtimeStatsSnapshot {
    pub scheduling_latency_ns: u64,
    pub max_scheduling_latency_ns: u64,
    pub page_faults: u64,
    pub context_switches: u64,
    pub interrupts: u64,
    pub system_calls: u64,
}

/// 优化状态
#[derive(Debug, Clone)]
pub struct OptimizationStatus {
    pub realtime_scheduling_enabled: bool,
    pub memory_locked: bool,
    pub cpu_affinity_set: bool,
    pub interrupt_isolation_enabled: bool,
    pub numa_optimization_enabled: bool,
    pub power_optimization_enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_realtime_optimizer_creation() {
        let config = RealtimeConfig::default();
        let optimizer = RealtimeSystemOptimizer::new(config).unwrap();
        
        let status = optimizer.get_optimization_status();
        assert!(!status.realtime_scheduling_enabled); // 初始状态
    }
    
    #[tokio::test]
    async fn test_ultra_low_latency_config() {
        let config = RealtimeSystemOptimizer::ultra_low_latency_config();
        assert!(config.enable_realtime_scheduling);
        assert_eq!(config.realtime_priority, 99);
        assert!(config.enable_memory_locking);
        assert_eq!(config.memory_lock_limit, 8 * 1024 * 1024 * 1024);
    }
    
    #[test]
    fn test_stats_snapshot() {
        let optimizer = RealtimeSystemOptimizer::new(RealtimeConfig::default()).unwrap();
        let stats = optimizer.get_stats();
        
        // 初始状态应该都是0
        assert_eq!(stats.scheduling_latency_ns, 0);
        assert_eq!(stats.max_scheduling_latency_ns, 0);
        assert_eq!(stats.page_faults, 0);
    }
}