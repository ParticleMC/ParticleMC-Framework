<!-- Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
SPDX-License-Identifier: GPL-3.0-or-later -->
# 性能基准与调优说明（T15 / T16）

本文档记录自研 ECS 内核 `minestom-ecs`（零外部依赖、纯 `std`）的性能基准结果，以及为「极致压榨」所落地的优化手段（SIMD / LTO / PGO / 线程亲和 / 无分支）与其已知约束。

> **测量环境说明**
> - 所有基准均在本机运行，同时给出 **`dev`** 与 **`release`**（`lto="fat"` 等已生效）两套数值；`release` 为生产真实表现。
> - 关键认知（已实测）：`release` 的 **10–15× 吞吐加速**来自 LTO 跨 crate 内联 + 静态查询展开的总收益；而**手写 SIMD 在规整 `f32` 加法上并不优于 LLVM 自动向量化**（见第 2 节修正结论），PGO 亦然（见 `docs/pgo.md` 第 5 节）。
> - 本机为 x86_64，AVX2 可用；非 x86_64 平台的 SIMD 路径走标量回退（见 `docs/` 其余文档）。

---

## 1. 吞吐基准（T15.3）

基准实现：`crates/minestom-ecs/benches/throughput.rs`（`[[bench]] harness=false`，零外部依赖，`std::time::Instant` 计时）。
运行命令：
```powershell
cargo test --bench throughput -- --nocapture            # dev
cargo test --release --bench throughput -- --nocapture   # release（lto="fat" 等已生效）
```
**固定规模 `N = 10000` 个实体（`Position` 组件），预热后取均值。下表同时给 dev 与 release（release 为生产真实表现）。**

| 基准项 | 含义 | dev per_op | **release per_op** | release 加速 | 吞吐估算 (release) |
| --- | --- | --- | --- | --- | --- |
| `spawn_throughput` | 批量构建 10000 实体（含 Archetype 匹配） | ~22.40 ms | **2.14 ms** | ~10.4× | ~4.66 M entities/s |
| `query_iter` | 对 10000 实体静态 `Query<(&Position,)>` 只读迭代 | ~3.08 ms | **198.5 µs** | ~15.5× | ~50.4 M entities/s |
| `single_tick` | 单实例 World 一次完整 schedule（物理系统） | ~4.50 ms | **305.4 µs** | ~14.7× | ~3.27 K ticks/s（单实例） |
| `migration_cost` | 跨 World 迁移 10000 实体 + 重跑 schedule | ~65.08 ms | **6.35 ms** | ~10.2× | — |

> 测量地点：`bench_run_thr.log`（dev）、`bench_run_thr_rel.log`（release）。

观察：
- `release`（`lto="fat"` + `codegen-units=1` + `opt-level=3`）带来 **10–15× 整体加速**，证明 T16.3 的极致编译期调优真实生效；这是「静态查询展开 + 无分支 SoA 遍历 + 跨 crate 内联」叠加后的总收益。
- `query_iter` 远高于 `spawn_throughput` 的「每实体」成本，因为 spawn 包含 Archetype 分配与组件写入，`query_iter` 仅为顺序 SoA 遍历。
- `migration_cost` 最昂贵：迁移需从源 World 拆解、在目标 World 重建 Archetype 并搬运组件数据；这是跨世界语义的固有成本，后续可作为专项优化（如共享只读组件零拷贝）。

> **与 旧 ECS 方案 基线对比**：本仓库在迁移后已移除 旧 ECS 方案 依赖（见 `docs/decisions.md` IC-12），当前环境无法获取可对比的 旧 ECS 方案 基线数值。如需对比，应在同机型同 `release` profile 下单独拉取 旧 ECS 方案 重跑等价基准，本表不臆造对比值。

---

## 2. 优化对比基准（T16.1 / T16.2 / T16.5）

基准实现：`crates/minestom-ecs/benches/optimizations.rs`。
运行命令：
```powershell
cargo test --bench optimizations -- --nocapture            # dev
cargo test --release --bench optimizations -- --nocapture   # release（lto="fat" 等已生效）
```
**dev profile 仅反映「策略相对差距」；release 才是生产真实表现。下表同时给出两者（N 同 dev）。**

| 对比项 | 含义 | dev per_op | **release per_op** | release 相对 |
| --- | --- | --- | --- | --- |
| `simd_scalar_add` | 标量批量 `Position += delta`（LLVM 自动向量化） | ~67.48 µs | **878 ns** | 基线 |
| `simd_avx2_add` | 手写 AVX2 `_mm256_*` 批量更新（x86_64，U4） | ~71.19 µs | 897 ns | 仍慢 ~2% |
| `static_query_iter` | 编译期展开的静态 `Query` 迭代 | ~2.92 ms | **188.9 µs** | vs runtime **2.43× 更快** |
| `runtime_get_per_entity` | 逐实体 `world.get::<T>()` 运行时查找 | ~5.99 ms | 459.3 µs | 基线 |
| `column_iter_branchless` | SoA 列 `iter()` 无分支遍历（U1） | ~24.48 µs | 4.70 µs | 与分支版持平 |
| `column_get_branching` | 逐元素 `get(i)` 带边界检查 | ~81.09 µs | 4.55 µs | 差距消失 |

> 测量地点：`bench_run_opt.log`（dev）、`bench_run_opt_rel.log`（release）。

关键结论（release 实测，已用 `cargo test --release` 复核）：
- **静态查询展开（T16.1）✅ 成立且更强**：release 下静态 `Query` 比运行时 `get` 快 **2.43 倍**（dev 为 2.05×）。这是编译期确定 archetype、消除逐实体哈希查找的实打实收益，是热路径首要加速面。
- **SIMD（T16.2）⚠️ 实测不优于自动向量化（重要修正）**：release 下 `simd_avx2_add`（897 ns）与 `simd_scalar_add`（878 ns）**基本持平、AVX2 版反而慢 ~2%**。根因：LLVM 对 `add_assign_scalar` 这种规整 `f32` 加法循环**已自动向量化为等价 AVX2**，手写 intrinsic 未带来额外收益，反而因 `#[target_feature(enable="avx2")]` 函数不可跨调用内联 + `loadu/storeu` 非对齐存取而略有开销。**该路径保留意义**：(a) 作为 U4 白名单 unsafe 面的可工作范例与正确性验证（`simd_add_matches_scalar` 数值一致性测试保证）；(b) 在更复杂的多组件/非规整访存核上可能显现收益（待专项基准）。**不应期望在简单 element-wise 加法上获得加速**。
- **无分支遍历（T16.5 / U1）⚠️ release 下差距消失（重要修正）**：dev 下无分支比分支快 3.3×（避免逐元素边界分支），但 release 下 `column_iter_branchless`（4.70 µs）与 `column_get_branching`（4.55 µs）**基本持平、分支版反略快**。根因：LLVM 在 release 下已对 `get(i)` 边界检查做分支预测/证明优化，规整连续访问下无分支优势被抵消。**无分支设计仍保留**：其真实价值在「不规则/动态索引访问」「与 U1 裸指针迭代器复用同一套推进逻辑（安全护城河）」等场景；连续规整遍历上带边界检查的 `get` 经优化后等价。


---

## 3. 零分配断言（T15.1）

实现：`crates/minestom-ecs/tests/zero_alloc.rs::hot_path_query_is_zero_allocation`。
机制：自定义 `global_allocator` 计数，在预热后进入热循环，对 1024 实体做「`query::<(&Position,)>` 只读迭代 + `query_mut::<(&mut Position,)>` 原地 `pos.x += 1.0`」，循环前后断言 `ALLOC_COUNT == 0`，并验证落盘 `x == 1.0 + HOT_ITERATIONS`。
**结果：lib 测试 188 passed（含本项），确认单 tick 热路径零堆分配。**

---

## 4. 缓存行对齐断言（T15.2）

实现：`crates/minestom-ecs/src/storage/soa.rs::column_container_aligned_and_no_false_sharing`。
断言：
- `SoAColumn<T>` 容器 `align_of == 64`（缓存行对齐类型 `Align64` 包裹）；
- 数组中相邻列的基址差 ≥ 64 字节，可达「无伪共享」属性。

**受限项（重要偏差记要）**：受 stable 工具链 `allocator_api` 仍 `E0658` 不稳定、且本 crate 级 `#![deny(unsafe_code)]` 约束，无法为元素级 `Vec<f32>` 缓冲提供自定义 64 字节对齐分配器。因此**元素缓冲的 64 字节对齐未启用**，仅保证「列容器级对齐 + 相邻列无伪共享」（真实可达属性）。元素缓冲对齐优化需待 `allocator_api` 稳定或引入内联对齐分配器（需局部 `unsafe` 豁免，已记入白名单 U4）后落地。详见 `docs/decisions.md`。

---

## 5. 极致压榨手段落点（T16 其余子项）

| 子项 | 落点文件 | 说明 |
| --- | --- | --- |
| T16.2 SIMD | `crates/minestom-ecs/src/storage/soa.rs` `SoAColumn<f32>::add_assign_simd` | `#[target_feature(enable="avx2")]` + `loadu_ps/storeu_ps`；非 x86_64 经 `add_assign_scalar` 回退。白名单 U4。 |
| T16.3 Release | 根 `Cargo.toml` `[profile.release]` | `lto="fat"`、`codegen-units=1`、`panic="abort"`、`opt-level=3`、`strip=true`。PGO 见 `docs/pgo.md` + `build-pgo.{ps1,sh}`。 |
| T16.4 亲和 | `crates/minestom-ecs/src/util.rs` `set_current_thread_affinity` + `scheduler.rs::tick_all` | 零依赖线程亲和（Win `SetThreadAffinityMask` / Linux `sched_setaffinity` / macOS 静默降级）。NUMA 见 `docs/affinity-numa.md`。 |
| T16.6 校验 | `.github/workflows/ci.yml` + `docs/sanitizers.md` | stable 构建测试 + nightly Miri + Linux valgrind；本机 Miri 可跑。 |

---

## 6. 复跑指引

```powershell
# 吞吐基准
cargo test --bench throughput -- --nocapture
# 优化对比基准
cargo test --bench optimizations -- --nocapture
# 零分配 / 对齐 / SIMD 正确性
cargo test -p minestom-ecs --lib
# release 下验证 SIMD/LTO 真实收益
cargo test --release --bench optimizations -- --nocapture
```
