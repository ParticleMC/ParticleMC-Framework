# PGO 构建与极致 Release 调优（T16.3）

本文档说明如何对自研 ECS 内核 `minestom-ecs` 启用 **PGO（Profile-Guided Optimization，基于性能剖析的优化）**，以及仓库根 `Cargo.toml` 中 `[profile.release]` 各开关的取舍。

> 前置阅读：`docs/benchmarks.md`（基准数值）、`docs/affinity-numa.md`（运行时压榨）。

---

## 1. 当前 Release Profile（已落地）

根 `Cargo.toml`：

```toml
[profile.release]
lto = "fat"          # 跨 crate 全量链接期优化（最大化内联与死代码消除）
codegen-units = 1    # 单代码生成单元，换取更激进的跨函数优化（代价：编译更慢）
panic = "abort"      # 发布构建中以 abort 替代 unwind，去除 panic 展开表，体积更小、分支更少
opt-level = 3        # 最高优化等级
strip = true         # 剥离符号，减小二进制体积
```

取舍说明：
- `lto="fat"` + `codegen-units=1` 是「极致压榨」组合，会显著拉长编译时间，但能让 `Query` 静态展开、无分支 SoA 遍历、跨 crate 内联真正生效——本仓库实测 `release` 比 `dev` 在吞吐基准上快 **10–15×**（见 `docs/benchmarks.md` 第 1 节）。仅在发布构建使用。
- `panic="abort"` 与项目章程「生产代码禁 `unwrap`/`expect`」一致：发布路径不再依赖 panic 展开；开发/测试构建保持默认 `unwind` 以便测试框架捕获断言。
- **关于 SIMD 的修正认知**（`docs/benchmarks.md` 第 2 节已实测）：`release` 下 `add_assign_simd` 的 8 路 `f32` 并行**并未**跑赢 `add_assign_scalar`——LLVM 已将规整 `f32` 加法循环自动向量化为等价 AVX2。`lto="fat"` 的收益主要来自「跨 crate 内联 + 死代码消除 + 静态查询展开」，而非手写 SIMD。手写 SIMD 路径保留作 U4 白名单 unsafe 范例。务必用 `--release` 复跑 `optimizations` 基准核对此结论。

---

## 2. PGO 是什么

PGO 分两阶段：
1. **插桩构建（instrument）**：编译时插入轻量计数器，记录真实运行期的分支/调用热点。
2. **训练运行（train）**：用代表性负载跑插桩二进制，产出 `*.profdata` 剖析文件。
3. **优化构建（optimize）**：用剖析数据重新编译，让编译器把热路径放满、冷路径瘦身、间接调用去虚化。

对 ECS 而言，训练负载应覆盖「实体 spawn、静态 Query 迭代、tick 调度、跨 World 迁移」——即 `benches/throughput.rs` 与 `benches/optimizations.rs` 的组合，再以典型游戏循环驱动若干 tick。

---

## 3. 一键构建脚本

仓库提供跨平台脚本，自动完成「插桩 → 训练 → 优化构建」：

- Windows：`build-pgo.ps1`
- Linux/macOS：`build-pgo.sh`

用法（以 server 二进制为例，按需替换 `--bin` 目标）：

```powershell
# Windows
.\build-pgo.ps1

# Linux / macOS
./build-pgo.sh
```

脚本关键步骤（以 Linux 为例，Windows 等价见脚本）：
```bash
# 1. 插桩构建
RUSTFLAGS="-Cprofile-generate=$PWD/pgo-data" \
  cargo build --release --target-dir pgo-target

# 2. 训练运行（代表性负载）
./pgo-target/release/<bin>  # 或跑基准: cargo test --release --bench throughput

# 3. 合并剖析
llvm-profdata merge -o pgo-data/merged.profdata pgo-data

# 4. 优化构建（最终产物）
RUSTFLAGS="-Cprofile-use=$PWD/pgo-data/merged.profdata" \
  cargo build --release --target-dir final-target
```

---

## 4. 平台边界与前置依赖

| 平台 | 支持度 | 说明 |
| --- | --- | --- |
| Linux x86_64 | ✅ 推荐 | `rustc` 默认使用 LLVM 后端，原生支持 `profile-generate` / `profile-use`；`llvm-profdata` 随 Rust 工具链（或系统 LLVM）提供。 |
| Windows x86_64 | ⚠️ 视工具链 | MSVC 后端 PGO 走 `link.exe /GENPROFILE`；若用 `rustc` 的 LLVM 后端则同 Linux。脚本默认走 LLVM 路径。 |
| macOS | ⚠️ 需 Xcode 工具链 | 需 `xcrun llvm-profdata` 或对应 LLVM；Apple Clang 与 Rust LLVM 剖析格式可能不互通，建议确认工具链一致性。 |
| 非 x86_64（aarch64 等） | ✅ 原理通用 | 仅 SIMD 路径回退标量（见 `docs/benchmarks.md` 第 2 节），PGO 本身与架构无关。 |

前置：
- 需要 Rust 1.??+（LLVM PGO 稳定可用）；`rustc --version` 确认。
- `llvm-profdata` 需在 `PATH`（`rustup component add llvm-tools-preview` 可获取随附版本）。

---

## 5. 验证 PGO 收益

PGO 后复跑 `docs/benchmarks.md` 第 1、2 节基准，重点观察：
- **整体 tick / 查询路径的绝对耗时下降**：PGO 通过热点分区与间接调用去虚化，主要改善「跨函数 / 多系统调度」类开销（如 `InstanceScheduler::tick_all`、Archetype 路由），而非 micro-kernel 算术。
- **`static_query_iter` 应保持对 `runtime_get_per_entity` 的显著领先**（release 实测 **2.43×**），PGO 通常进一步放大该差距。
- ⚠️ **关于手写 SIMD 的修正预期**：release 实测 `simd_avx2_add`（897 ns）与 `simd_scalar_add`（878 ns）**基本持平**——因为 LLVM 已将规整 `f32` 加法循环自动向量化为等价 AVX2，手写 intrinsic 不提供额外收益。PGO 不会改变这一等价关系。手动 SIMD 路径保留作 U4 白名单 unsafe 范例与更复杂核（非规整访存 / gather-scatter）的潜在加速面，不应期望在 element-wise 加法上跑赢自动向量化。
- ⚠️ **关于无分支遍历**：release 下 `column_iter_branchless` 与 `column_get_branching` 已持平（LLVM 优化了边界检查分支），PGO 同样不会改变该等价关系。

若 PGO 后整体 tick 路径耗时未下降，检查：训练负载是否覆盖真实热路径、剖析文件是否正确合并并被 `profile-use` 读取。
