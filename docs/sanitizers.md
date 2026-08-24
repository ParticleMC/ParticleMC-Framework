# 内存与未定义行为校验（T16.6）

本文档说明如何对 `minestom-ecs` 内核施加内存安全/未定义行为（UB）校验：nightly **Miri**（本机可跑）、Linux **valgrind**，以及它们在 CI 中的落点（`.github/workflows/ci.yml`）。

> 背景：本 crate 级 `#![deny(unsafe_code)]`，唯一豁免面是白名单 U4（`util.rs` 线程亲和 extern 调用、`soa.rs` SIMD loadu/storeu）。即便如此，SIMD 的 `loadu/storeu` 仍需验证「越界读取/对齐假设」，Miri 是关键防线。

---

## 1. Miri（Rust 中端解释器，检测 UB）

Miri 在解释执行下捕获：越界访问、数据竞争、未初始化内存读取、无效裸指针、违反 `&mut` 别名规则等。

前置：
```bash
rustup toolchain install nightly
rustup component add miri --toolchain nightly
cargo +nightly miri setup
```

运行（本机推荐）：
```bash
# 校验全部 lib 单元测试（含零分配/对齐/SIMD 断言）
cargo +nightly miri test -p minestom-ecs --lib

# 仅校验 unsafe 面（SIMD / 亲和）
cargo +nightly miri test -p minestom-ecs --lib simd_add_matches_scalar
```

注意：
- Miri 默认禁用部分可能 UB 的 intrinsic；SIMD loadu/storeu 在 Miri 下以标量模拟，可验证「语义正确性」但非「向量化性能」。
- Miri 对 `std::thread` 数据竞争的检出需要 `-Zmiri-disable-isolation` 关闭部分隔离；CI 中已加相应 flag。
- 运行耗时远大于原生测试，仅对核心断言使用，不跑大规模 bench。

---

## 2. valgrind（Linux，检测泄漏/越界/条件跳转）

valgrind 的 Memcheck 检测：未初始化读取、非法读写、内存泄漏。

前置（Debian/Ubuntu）：
```bash
sudo apt-get install -y valgrind
```

运行（Linux）：
```bash
# 用 release 测试二进制跑 valgrind
cargo test --no-run --release -p minestom-ecs
valgrind --error-exitcode=1 --leak-check=full \
  ./target/release/deps/minestom_ecs-<hash> --lib -- --nocapture
```

边界：
- valgrind 对 SIMD 指令兼容性良好（解释执行 SSE/AVX），可覆盖 loadu/storeu 的越界路径。
- macOS 无 valgrind（仅 `sanitizers`/Instruments 替代），故 CI 仅在 Linux 跑 valgrind，macOS 走 Miri（nightly）。

---

## 3. CI 接法（`.github/workflows/ci.yml`）

工作流分三个 job：
1. `stable`：stable 工具链 `cargo build --workspace` + `cargo test -p minestom-ecs --lib`（门禁，与本地一致）。
2. `miri`：nightly + `cargo miri test -p minestom-ecs --lib`，校验 UB。
3. `valgrind`：仅 `ubuntu-latest`，`apt` 装 valgrind 后跑 Memcheck。

> 本仓库当前未托管于 GitHub（无 `.github/`），该 workflow 文件随代码提交，待接入 CI 平台（GitHub Actions / 自托管 runner）后自动生效；在 Windows 开发机上可本地跑 `stable` 与 `miri`（WSL2 提供 Linux valgrind 环境）。

---

## 4. 已知校验盲区

- **PGO 二进制**：插桩/优化构建产物不纳入 Miri/valgrind（它们是优化后产物，语义等同，由对应 `release` 测试兜底）。
- **SIMD 性能路径正确性**：Miri 仅验语义，不验「AVX2 实际生成」；AVX2 真实向量化由 `cargo test --release --bench optimizations` 的反汇编/`perf` 复核（见 `docs/pgo.md` 第 5 节）。
- **线程亲和 extern 调用**：`set_current_thread_affinity` 的 Windows/Linux 系统调用无法在 Miri 下完整模拟，仅验证「掩码计算逻辑」与「降级分支返回 false」；真实绑定行为靠 `docs/affinity-numa.md` 第 5 节手动验证。
