# 更新日志

## [3.3.6] - 2025-01-30

### 新增
- **Stellium SWQOS 支持**：全新 Stellium 客户端实现
  - 使用标准 Solana `sendTransaction` RPC 格式
  - 自动连接保活，60 秒 ping 间隔
  - 5 个小费账户用于负载分配
  - 支持 8 个区域端点（纽约、法兰克福、阿姆斯特丹、东京、伦敦等）
  - 最低小费要求：0.001 SOL

### 变更
- **更新最低小费要求**以提高交易成功率：
  - NextBlock: 0.00001 → 0.001 SOL
  - ZeroSlot: 0.00001 → 0.001 SOL
  - Temporal: 0.00001 → 0.001 SOL
  - BloxRoute: 0.00001 → 0.001 SOL
  - FlashBlock: 0.00001 → 0.001 SOL
  - BlockRazor: 0.00001 → 0.001 SOL
- 增强异步执行器，添加小费验证警告
