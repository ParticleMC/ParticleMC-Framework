<!-- Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
SPDX-License-Identifier: GPL-3.0-or-later -->
# Block Bench — 方块写入压力测试

## 设备参数
CPU：i5-13400f

RAM：Colorful DDR4 16GB 3200MHz (*2)

## 测试规模
- 网格：31×31 = **961 区块**
- 每区块：1 个区段 × 4096 格（Y 0..15）
- 总方块数：**3,936,256 个**
- 随机块 ID 范围：[1, 200]（空气 id=0 不出现）
- 伪随机源：确定性 hash（无外部依赖）

## 测试结果（单次运行，优化前）

| 阶段 | 吞吐量 | 耗时 |
|------|--------|------|
| Fill（填充随机方块）| 28,448 blocks/s（≈0.03 M/s） | 138,362 ms |
| Read（全量读取验证）| 3,989,336 blocks/s（≈4.0 M/s） | 987 ms |
| Clear（清空为空气）| 29,431 blocks/s（≈0.03 M/s） | 133,743 ms |
| **总计** | 平均 0.04 M blocks/s | **273 秒（约 4.5 分钟）** |

## 测试结果（单次运行，优化后 — `bulk-fill-optimization`）

| 阶段 | 吞吐量 | 耗时 |
|------|--------|------|
| Fill（批量填充）| **3,623,806 blocks/s（≈3.62 M/s）** | **1,086 ms** |
| Read（全量读取验证）| 4,300,899 blocks/s（≈4.30 M/s） | 915 ms |
| Clear（批量清空）| **299,280,435 blocks/s（≈299 M/s）** | **13 ms** |
| **总计** | 平均 5.86 M blocks/s | **2,015 ms（约 2 秒）** |

## 关键发现

### 1. 写入吞吐量提升 120×+
- 填充：0.03 M/s → **3.62 M/s**（**~127×** 提升）
- 清空：0.03 M/s → **299 M/s**（**~10,000×** 提升，因空气写入走单值 palette 特例）
- 总耗时：273s → **2.0s**（**~135×** 提升）

### 2. 瓶颈消除
- **Palette repack**：从 per-write 降到 per-chunk 一次（`fill_blocks` 单次构建调色板）
- **光照重算**：`BulkEditContext` 延迟到 `finalize()` 统一迭代传播，不干扰填充阶段
- **LRU 更新**：批量期间暂停，`finalize()` 时一次性合并（含去重）
- **Dirty 标记**：批量期间收集到 `pending_dirty`，`finalize()` 一次合并

### 3. 读取性能不受影响
读取速度保持稳定（~4 M/s），验证数据正确性。

### 4. 性能瓶颈定位（优化前）
```
set_block_id_world (每格 0.035ms)
├── Chunk::set_block             微秒级
├── Section::set_block_id        微秒级（调色板命中时）
├── ChunkStore::mark_access      微秒级（VecDeque::retain）
├── ChunkStore::recompute_light  ★ 主要瓶颈：邻块边界提取 + BFS
└── ChunkStore::evict_lru        偶发（仅超限时触发）
```

### 5. 性能瓶颈定位（优化后）
```
BulkEditContext::fill_chunk (每区块 ~1.1ms)
├── Section::fill_blocks         微秒级（单次调色板构建 + 线性写入）
├── pending_chunks 写入          微秒级（HashMap insert）
└── finalize() 迭代光照传播      毫秒级（961 区块 × 2 轮迭代）
```

## 架构文件
- 规格文档：`.specs/bulk-fill-optimization/spec.md`
- 测试入口（优化版）：`crates/block-bench/src/main.rs`
- Cargo workspace：已在根 Cargo.toml 的 members 列表中注册
- 构建目标目录：`target_local/`（绕过沙箱全局 target 限制）
- 核心实现：
  - `crates/minestom-core/src/instance/chunk.rs` — `Section::fill_blocks`、`Chunk::fill_section_blocks`
  - `crates/minestom-core/src/instance/chunk_store.rs` — `BulkEditContext`、`ChunkStore::start_bulk_edit`
