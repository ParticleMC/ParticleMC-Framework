<!-- Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
SPDX-License-Identifier: GPL-3.0-or-later -->
# 线程亲和与 NUMA 感知（T16.4）

本文档说明自研 ECS 调度器 `InstanceScheduler` 的线程亲和落地方式、NUMA 感知的必要性与平台边界。

> 前置阅读：`docs/pgo.md`（编译期压榨）、`docs/benchmarks.md`（基准）。

---

## 1. 亲和落地点

- **API**：`crates/minestom-ecs/src/util.rs::set_current_thread_affinity(core: usize) -> bool`
- **接入点**：`crates/minestom-ecs/src/scheduler.rs::InstanceScheduler::tick_all`
  - 经 `std::thread::available_parallelism()` 取逻辑核心数 `cores`；
  - 每个 `thread::scope` worker 在 `config.affinity == true` 时执行 `set_current_thread_affinity(chunk_index % cores)`。
- **配置开关**：`SchedulerConfig.affinity`（默认 `false`，需显式开启）。

设计动机：多实例 World 并行 tick 时，把每个 worker 绑定到固定核心，可避免 OS 调度在核心间迁移线程导致的缓存/TLB 抖动；配合 SoA 缓存行对齐（见 `docs/benchmarks.md` 第 4 节），降低伪共享与冷启动代价。

---

## 2. 零依赖实现（白名单 U4）

`set_current_thread_affinity` 通过**原始 `extern` 声明**直接调用系统 API，不引入任何外部 crate（符合项目「零外部依赖」内核约束）：

| 平台 | 系统调用 | 行为 |
| --- | --- | --- |
| Windows | `kernel32!GetCurrentThread` + `SetThreadAffinityMask` | 按 `1usize << core` 设置亲和掩码 |
| Linux（glibc） | `sched_setaffinity(pid=0, ...)` | 经 `u64` 掩码绑定核心 |
| macOS / 其他 | — | 静默降级，返回 `false`，不影响调度正确性 |

`#[allow(unsafe_code)]` 仅作用于该函数局部（crate 级 `#![deny(unsafe_code)]` 仍封死其余区域）；属白名单 U4（SIMD/系统调用面）。

---

## 3. NUMA 感知（建议，非强制）

在双路/多路服务器（如 2× Xeon、多 CCD 的 EPYC/Threadripper）上，内存被划分为多个 NUMA 节点，**跨节点访存延迟显著高于本节点**。对 ECS 的意义：

- 实体数据（SoA 缓冲、Archetype 表）应优先分配在「运行该实例 tick 的线程所属 NUMA 节点」本地内存；
- 否则「线程绑核（亲和）」与「数据远程分配（NUMA 错配）」会相互抵消收益，甚至因跨节点带宽成为新瓶颈。

現状与边界：
- 当前 `set_current_thread_affinity` 仅做**核心绑定**，未做 NUMA 内存策略（`libnuma`/`set_mempolicy`/`VirtualAllocExNuma`）——这是刻意的最小化实现：
  - Rust stable 无跨平台 NUMA 内存 API，引入则需额外 unsafe 面与平台分支；
  - 单机/单 NUMA 节点部署（绝大多数开发机、单路服务器）下 NUMA 错配不存在，绑定即可获益。
- **多 NUMA 节点生产部署建议**：在启动期用 `numactl --cpunodebind=<node> --membind=<node> ./server`（Linux）或等价 Windows 组策略，将整个实例进程钉到单个 NUMA 节点；若需实例级跨节点分片，再考虑在 `util.rs` 增加 NUMA 节点查询 + `set_mempolicy` 落点（届时扩展白名单）。

---

## 4. 平台边界总表

| 能力 | Windows | Linux | macOS | 其他 |
| --- | --- | --- | --- | --- |
| 核心亲和（`set_current_thread_affinity`） | ✅ | ✅ | ⚠️ 降级 | ⚠️ 降级 |
| NUMA 内存策略 | ⚠️ 需显式 `VirtualAllocExNuma`/组策略 | ⚠️ 需 `numactl`/`set_mempolicy` | ❌ | ❌ |
| 降级语义 | 不支持平台静默返回 `false`，调度功能不受影响 | 同左 | 同左 | 同左 |

> 降级**不等于错误**：在 macOS 或不支持的平台开启 `config.affinity` 仅是不生效，tick 仍正确并行。

---

## 5. 验证

- 正确性：`cargo test -p minestom-ecs --lib` 已覆盖 scheduler 并行 tick 路径（含 `affinity` 开关分支）。
- 行为验证（手动）：在 Linux 用 `taskset -c 0-3 ./server` 或开启 `affinity` 后，经 `htop`/`perf` 观察 worker 线程是否稳定落于指核心；在 Windows 用 Process Explorer 的「Set Affinity」对照。
- Miri/内存安全：见 `docs/sanitizers.md` 与 `.github/workflows/ci.yml`。
