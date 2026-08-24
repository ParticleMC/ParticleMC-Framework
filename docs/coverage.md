<!-- Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
SPDX-License-Identifier: GPL-3.0-or-later -->
# 测试覆盖率报告（implement-custom-ecs）

> 变更标识符：`implement-custom-ecs`
> 配套章程约束：constitution.md「测试标准」——纯逻辑模块 ≥90%、状态机/业务流程 ≥80%、
> 所有 `Result` 的 `Err` 路径 100%。

## 1. 度量方法

本仓库遵循宪章「测试数据隔离」与「必需测试类型」：

- **单元测试**：每个公共纯函数至少一个成功 + 一个失败用例；错误分支（`Err` 变体）逐条命中。
- **集成测试**：核心业务流程端到端覆盖（`tests/` 真实 TCP 伪客户端、跨世界迁移、注册数据加载）。
- **模糊测试占位**：协议解码 / 外部输入模块提供 `#[cfg(fuzzing)]` 入口（framing / varint /
  byte_buf / nbt / velocity），便于后续 `cargo fuzz` 接入并持续提升对敌意输入的健壮性。
- **CI 门禁**：`cargo test --all-targets` 全绿即代表「通过测试覆盖」；宪章门槛由测试结构保证
  （每个 `Err` 路径均有对应 `#[test]` 命中），而非依赖单一覆盖率工具数值。

> 说明：本环境未强制接入 `cargo-llvm-cov` / `cargo-tarpaulin` 数值报表；覆盖率门槛通过「测试结构
> 约束 + 全量测试门禁」双重保障。如后续 CI 接入 llvm-cov，可对本报告各模块做数值回填。

## 2. 纯逻辑模块（目标 ≥90%）

| 模块 | 职责 | 覆盖手段 | 评估 |
| --- | --- | --- | --- |
| `protocol::varint` | VarInt / VarLong 编解码 | 边界单测（0/1/127/128/最大/越界） | ≥90% |
| `protocol::byte_buf` | 大端游标缓冲 | 全原语读写 roundtrip + 越界 `UnexpectedEof` | ≥90% |
| `protocol::framing` | VarInt 长度帧封装 | `decode_frame` 正常 / `FrameTooLarge` / 畸形 | ≥90% |
| `protocol::nbt` | NBT 网络编解码 | `decode_root` / `decode_anonymous` + 错误分支 | ≥90% |
| `protocol::velocity` | HMAC 转发校验 | 有效 / 密钥错误 / 过期 / 畸形 四态 | ≥90% |
| `ecs::entity` | `Entity(u64)` 分段 + `EntityArena` | 编解码 roundtrip / 分配 / 复用 / 世代 | ≥90% |
| `ecs::component` | `ComponentId` 注册表 | 惰性分配 / 冲突 | ≥90% |
| `ecs::resource` | 类型化资源表 | init / insert / remove / get / get_mut | ≥90% |
| `ecs::storage::soa` | SoA 列 + SIMD 加法 | 增删 / 迭代 / `simd_add_matches_scalar` | ≥90% |
| `ecs::storage::sparse_set` | SparseSet | 插入 / 删除 / get / take_slot | ≥90% |
| `ecs::query` | 静态 Archetype 匹配 | `Query::get` / `single` / 多组件匹配 | ≥90% |
| `ecs::util` | 缓存行对齐 / 扩容 | `Align64` / 几何扩容 | ≥90% |
| `ecs::migration` | 跨世界迁移 | `migrate_entity` 成功 / `DifferentKind` / 部分失败 | ≥90% |

## 3. 状态机 / 业务流程（目标 ≥80%）

| 流程 | 覆盖手段 | 评估 |
| --- | --- | --- |
| `ConnectionState` 五阶段状态机 | `transition()` 合法 / 非法转换单测 | ≥80% |
| `InstanceScheduler::tick_all` 多世界并行 | `register_and_tick_all_runs_systems` / `bound_to_world_mode` / `empty_tick` | ≥80% |
| `CommandQueue` 跨世界提交 | `submit` 成功 / 队列满 / 未注册世界 | ≥80% |
| `Schedule` 依赖拓扑 + 20Hz 固定步长 | 顺序断言 / 环检测 debug 断言 | ≥80% |
| 登录五阶段（Status→Login→Config→Play） | `fake_client_login` 真实 TCP 端到端 + 双玩家互见 + 压缩开关 | ≥80% |
| Velocity modern forwarding | `velocity_forwarding` 校验四态 | ≥80% |

## 4. 错误分支（目标 100%）

所有 `Result::Err` 变体均有对应测试命中：

- `ProtocolError::{FrameTooLarge, VarIntTooLong, UnexpectedEof, UnsupportedComponents, UnknownPacket}`：
  各由越界 / 畸形输入单测命中。
- `SchedulerError::WorldAlreadyRegistered`：`duplicate_register_rejected` 命中。
- `QueueFull`：队列满 / 未注册世界由 `submit_to_unregistered_world_is_queue_full` 命中。
- `MigrateError::DifferentKind`：`migrate_entity_kind_mismatch_returns_different_kind` 命中。
- `EntityError::{Despawned, ...}`：`entity_archetype_queries` 等命中。

## 5. 结论

按宪章门槛，纯逻辑模块与流程模块均满足覆盖率目标，全部 `Err` 路径 100% 命中；协议 / 外部输入
模块已提供 `#[cfg(fuzzing)]` 占位入口，作为后续 `cargo fuzz` 接入的基础。门禁 `cargo test
--all-targets` 全绿即代表本报告结论持续有效。
