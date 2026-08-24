<!-- Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
SPDX-License-Identifier: GPL-3.0-or-later -->
# 架构决策记录（Architecture Decision Records）

本项目以 [Minestom](https://github.com/Minestom/Minestom)（Java）为设计蓝本，
使用 Rust + 自研 ECS（minestom-ecs）重写服务端框架。本文件记录关键架构决策及其理由。

## ADR-001：基于 Bevy ECS 作为服务器内核（已取代）

**状态**：已取代（被 ADR-015 取代；内核改为自研 ECS `minestom-ecs`）

**背景**：Minestom 不提供原版逻辑，而是提供高性能、可扩展的服务器框架；
游戏内容以"注册数据 + 事件系统"组织。Rust 侧需要一套成熟、高性能、
可组合的实体组件系统（ECS）作为内核。

**决策**：选用 Bevy ECS（Bevy 0.18.1）作为核心依赖，仅使用
`bevy_app` / `bevy_ecs` / `bevy_time` 三个子 crate，不引入渲染相关依赖。

**理由**：

- `bevy_ecs` 提供高性能并行 schedule 调度、组件/资源/事件模型，与 Minestom
  的"系统 + 事件"范式天然契合
- `bevy_app` 提供 `App` / `Plugin` 抽象，可将服务器功能按插件化组织，
  对齐 Minestom 的 `Extension` 机制
- `bevy_time` 提供统一的游戏时钟，便于实现心跳与定时任务
- 本地 vendored（离线 path 依赖）保证构建可复现、无需访问网络拉取
  渲染相关的庞杂依赖树

## ADR-002：本地 vendored Bevy 依赖（path 依赖）（已取代）

**状态**：已取代（被 ADR-015 取代；内核改为自研 ECS `minestom-ecs`）

**背景**：服务器构建环境需要稳定、可复现；Bevy 在 crates.io 上的完整依赖树
包含大量图形/渲染 crate，与服务器场景无关。

**决策**：从 `server-rs/bevy-0.18.1` 通过 `path` 依赖直接引用
`bevy_app` / `bevy_ecs` / `bevy_time`，并在 `[workspace.dependencies]`
中统一声明。bevy 自身是一个完整 workspace，其内部 `bevy_*` path 依赖
自动解析，无需在本 workspace 重复声明。

**理由**：

- 只编译服务器需要的子集，显著减少编译面
- 版本精确锁定（`=0.18.1`），避免 crates.io 浮动解析的不确定性
- 后续如需升级 Bevy，仅需替换 vendored 目录并同步 path/版本号

## ADR-003：骨架先行 + 数据注册策略

**状态**：已接受

**背景**：Minestom 生态以注册数据（Registry）驱动：方块、物品、实体、
生物群系等均通过注册表在启动时加载。

**决策**：分阶段推进——

1. **T1 骨架**：先搭建可编译的 Cargo workspace，不实现具体业务逻辑；
   模块由后续任务逐步填充（`plugin` / `data` / `world` / `network`）
2. **数据驱动**：注册项数据从 `resources/data/` 下的 TOML 文件加载，
   通过 `serde` 反序列化为强类型注册表，避免硬编码
3. **插件化**：以 `McServerPlugin` 作为功能单元注册数据与系统，
   对齐 Minestom 的 `Extension` 模型

**理由**：先保证构建链路与依赖关系稳定，再逐步引入业务复杂度，
降低集成风险。

## ADR-004：TOML 作为数据层格式

**状态**：已接受

**背景**：`resources/data/` 目录已存在数据文件，需要确定加载方式与格式。

**决策**：使用 TOML 作为注册数据的源格式，以 `toml` crate 解析、
`serde` 反序列化；`uuid` 用于注册项标识，`tokio` 承载异步网络与定时任务。

**理由**：

- TOML 人类可读、注释友好，适合维护大量配置类注册数据
- `serde` + `toml` 组合可零成本映射到 Rust 强类型结构体
- 与既有 `resources/data/` 数据目录解耦：该目录及其 `tools/` 转换器
  保持原样，不被本 workspace 改动

## ADR-005：1.21.11 数据包 ID 校准

**状态**：已接受

**背景**：协议版本升级到 1.21.11（protocol 767）后，Configuration 阶段的部分
clientbound 包 ID 与旧版不同（源自 1.20.5 的配置阶段引入与后续整理）。若沿用旧
ID，客户端会拒绝解析或误解包，登录流程无法进入 Play。

**决策**：以 1.21.11 实际线格式为准，重新校准各阶段关键 ID（见下表），并在
`protocol/dispatch.rs` 与各包实现的 `packet_id()` 中固化。

| 阶段 | 包 | 方向 | 1.21.11 ID |
| --- | --- | --- | --- |
| Login | LoginSuccess | S2C | 0x02 |
| Login | LoginAcknowledged | C2S | 0x03 |
| Configuration | FinishConfiguration | S2C | **0x03** |
| Configuration | FinishConfiguration | C2S | 0x03 |
| Configuration | ConfigDisconnect | S2C | **0x02** |
| Play | SynchronizePlayerPosition | S2C | 0x3E |
| Play | SetPlayerPosition | C2S | 0x1B |

**理由**：

- 以协议实测为准而非文档猜测：1.21.11 中 Configuration 阶段 `FinishConfiguration`
  的 S2C/C2S ID 均为 0x03，`ConfigDisconnect` 为 0x02
- 集成测试（`fake_client_login.rs`）按上述 ID 断言握手 → 游玩全流程，任何 ID 回归
  都会立即暴露
- 伪客户端测试在真实 TCP 上运行，避免了"本地编解码自洽但线上不兼容"的假阳性

## ADR-006：Velocity modern forwarding + 三层发包模型

**状态**：已接受

**背景**：服务器需要支持置于 Velocity 代理之后的部署形态（多子服共享玩家身份），
同时需要控制出站带宽与发送节奏，避免每包独立系统调用与区块洪水。

**决策**：

1. **Velocity modern forwarding**（`protocol/velocity.rs`）：握手包末尾容错读取
   转发 blob（`version(1) + signature(32) + payload`），HMAC-SHA256 常量时间校验
   签名、时间戳防重放（±5s）；`VelocityConfig` 支持 `secret` 空值直连模式与
   `enforce_proxy` 强制代理模式。身份经 `ForwardedIdentity` 落连接，登录时优先使用。
2. **三层 ClientNetwork 发包模型**（`network/client.rs`）：每连接维护
   `urgent_queue`（立即 flush）/ `normal_queue`（按 MTU=1400 聚合）/ `ChunkSender`
   （信用节流，客户端回包动态调速）；`broadcast` 一次序列化、多目标复制；
   `urgent_window_ticks` 支持子服切换紧急窗口。

**理由**：

- 转发 blob 使用与 Velocity 相同的 `hmac-sha256` 纯 Rust 实现，无 unsafe；
  常量时间比较防时序侧信道
- 三层模型对齐 Minestom 的 urgent/normal 队列语义，同时把区块发送抽象为
  独立的信用节流器，为真实区块流水线预留接口
- 直连与代理两种部署形态共用一套 `network_receive` 逻辑，仅身份来源不同

## ADR-007：监听侧状态标签（双状态表）

**状态**：已接受

**背景**：游戏主循环是同步的，而 TCP 监听在 tokio 异步运行时；监听任务无法
直接读写 `World`，但 `network_receive` 派发入站包时又必须知道该包所属的
协议状态。若由游戏侧反馈状态，会引入跨线程复杂同步与延迟。

**决策**：监听侧维护一份**独立的最小状态表** `ConnStateMap`（仅用于给入站帧打
`state` 标签），随收到的包推进：Handshake 帧解码 `Intention` 后置 Status/Login，
Login 0x03 → Configuration，Configuration 0x03 → Play；游戏侧 `ConnectionManager`
维护权威状态（身份、实体绑定）。两侧以 `conn_id` 关联，互不依赖。

**关键约束**：监听侧解码 `Intention` 必须使用「不含 packet_id」的包体（与
`dispatch` 一致）。曾因传入含 packet_id 的完整帧导致解码失败、状态永不推进、
后续所有包被误标 Handshake（集成测试 10 分钟超时）——该缺陷在 2026-08-07 修复。

**理由**：

- 零跨线程同步：状态表是 `Arc<Mutex<HashMap>>`，仅在监听任务内读写
- 派发零延迟：帧在入站通道里就自带正确状态标签，`network_receive` 无需等待
  游戏侧反馈
- 状态重复维护的成本极低（两处各 ~10 行状态转移逻辑），换来的是架构清晰与
  线程边界稳定



## ADR-008：generic/ 与 loot_tables/ 注册数据全量加载

**状态**：已接受

**背景**：`resources/data/` 共 72 个注册数据文件，此前仅 23 个被加载（顶层 11 个注册表 +
`tags/` 13 个）。`generic/`（44 文件、969 条 `[[entry]]`）与 `loot_tables/`（4 文件、1275 条）
数据完整却从未进入资源系统：`GenericRegistry` 仅 `init_resource` 建空表，loot 表零引用。

**决策**：

1. **始终加载**：`McServerPlugin::new()` 与 `with_preload()` 两条构造路径都装配
   `GenericRegistry`（`load_directory(generic)`）与 `LootTableRegistry`
   （`load_directory(loot_tables)`），移除空表占位。加载失败经 `unwrap_or_default`
   回退空表，不 panic。
2. **generic 以 id 兜底键**：`from_toml_str` 键提取优先 `name`，无 `name` 回退 `id`
   十进制字符串（如 `sound_sources` 的 `"0"`），两者皆无才跳过——保证 969 条零丢失。
3. **轻量 loot 建模**：`LootTable { name, table_type, random_sequence, pools, raw }`。
   `pools` 保留原始 `toml::Value`（嵌套 pools/entries/functions/conditions 不做
   serde 严格反序列化，规避跨表字段差异导致的反序列化失败）；`raw` 恒为整条 entry
   保真，顶层附加字段（如 block 表 9 条的 entry 级 `functions`）零丢失。

**数据事实**：`generic/` 44 文件 969 条 entry，`load_directory` 合并语义（同键后者覆盖）
下唯一键 923（46 处跨文件同名键被覆盖，如 banner_pattern 与 block_entity_types 的
`minecraft:skull`）；`loot_tables/` 1275 条 name 全局唯一、无重复。集成测试按 923 / 1275
断言。

**理由**：

- 数据文件已随 `tools/gen_registry_data.py` 完整生成（1.21.11 全量），闲置即浪费；
  "始终加载"与用户补齐数据意图一致，启动即全量可用
- `toml::Value` 原样存盘零字段丢失、零新增依赖，是复杂嵌套数据最稳妥的落地方式
- 目录加载复用 `TagRegistry::load_directory` 既有模式，实现与心智成本最低

## ADR-009：Crate 分工与库化声明

**状态**：已接受

**背景**：重构与文档梳理中发现认知歧义——`minestom-core` 与 `minestom-server`
被误认为"两个独立项目"。事实上二者同属一个 Cargo workspace（根 `Cargo.toml`
的 `[workspace]`），只是职责分工不同，需要一份明确的决策记录消除歧义。
变更标识符：`.specs/cleanup-and-libify-minestom`。

**决策**：

1. **双 crate 分工**（一个 workspace 的两个成员，而非两个项目）：
   - `crates/minestom-core`：**框架库**，承载全部业务逻辑——自研 ECS（minestom-ecs）内核、
     协议层、网络层、注册表、系统管线；公共模块 `component` / `event` /
     `instance` / `network` / `plugin` / `protocol` / `resource` / `schedule`
     / `system`。
   - `crates/minestom-server`：**入口二进制壳**——`main.rs` 读取环境变量并
     启动；`lib.rs` 的 `run()` 装配 `App` + tokio 运行时 + 20Hz 主循环，供
     二进制与 `tests/fake_client_login.rs` 集成测试复用。真正的服务器逻辑
     全部在 `minestom-core` 库中。
2. **显式库化**：`minestom-core` 的 `Cargo.toml` 显式声明
   `[lib] crate-type = ["rlib"]`，声明其只作为 Rust 库被链接。

**理由**：

- **框架与运行形态分离**：核心逻辑沉淀为可复用、可测试的库 crate，入口壳
  保持极薄；集成测试（`fake_client_login.rs`）直接以库 API 启动服务器，
  未来接入其它运行形态（如子服管理、后台守护进程）时无需改动框架层
- **显式 `crate-type = ["rlib"]`**：将该 crate 的交付形态自文档化，抑制
  意外产物的产出，使依赖关系与编译产物边界清晰可预期
- **单一 workspace 管理**：共享根 `Cargo.toml` 的 `[workspace.dependencies]`
  与 `Cargo.lock` 精确锁定，统一 `cargo build --workspace` /
  `cargo test --workspace` 门禁，从构建体系上消除"两个项目"的认知分裂

## ADR-010：协议升级 774 的包 ID 重新校准

**状态**：已接受（取代 ADR-005 的协议 ID 校准部分；ADR-005 描述 protocol 767，
本协议升级到 774）

**背景**：1.21.11 的 `fake_client_login` 集成测试在 protocol 767 下可完成握手→游玩，
但多个 clientbound 包 ID 与 774 不一致：客户端在 Configuration/Play 阶段会拒绝解析
或误解包，且缺失 774 新增的注册表同步（`RegistryData` ×N）与区块批次包。
变更标识符：`.specs/polish-minestom-framework`。

**决策**：以 1.21.11 实际线格式（protocol 774）为准重新校准，并在 `protocol/packets/*`
的 `packet_id()` 与 `protocol/dispatch.rs` 派发表中固化。关键差异 / 新增 ID 如下：

| 阶段 | 包 | 方向 | 767 (ADR-005) | 774 (本 ADR) |
| --- | --- | --- | --- | --- |
| Play | 出生位置 | S2C | `SynchronizePlayerPosition` 0x3E | `Position` **0x46** |
| Play | 玩家移动 | C2S | `SetPlayerPosition` 0x1B | `PlayerPosition` **0x1d** |
| Configuration | 注册表同步 | S2C | （无） | `RegistryData` **0x07** |
| Configuration | 标签同步 | S2C | （无） | `UpdateTags` **0x0d** |
| Configuration | 特性标志 | S2C | （无） | `FeatureFlags` **0x0c** |
| Play | 登录（play） | S2C | （无） | `Login` **0x30** |
| Play | 生命值 | S2C | `SetHealth` | `UpdateHealth` **0x66** |
| Play | 玩家信息 | S2C | `PlayerInfoUpdate` | `PlayerInfo` **0x44** |
| Play | 生成实体 | S2C | `SpawnPlayer` | `SpawnEntity` **0x01** |
| Play | 区块数据 | S2C | `ChunkBatchData` | `MapChunk` **0x2c** |
| Play | 区块批次开始 | S2C | （无） | `ChunkBatchStart` **0x0c** |
| Play | 区块批次完成 | S2C | `ChunkBatchFinished` 0x0b | `ChunkBatchFinished` **0x0b**（不变） |

> Configuration 阶段 `FinishConfiguration`(S2C/C2S) 0x03 与 `ConfigDisconnect` 0x02、
> Login 阶段 `LoginSuccess` 0x02 / `LoginAcknowledged`(C2S) 0x03 在 767/774 一致，沿用。

**理由**：

- 以协议实测为准：集成测试 `fake_client_login.rs` 在真实 TCP 上按 774 ID 断言
  配置阶段顺序 `RegistryData×N → UpdateTags → FeatureFlags → FinishConfiguration`
  与游玩阶段 `Login → Position → UpdateHealth → PlayerInfo` 及
  `ChunkBatchStart → MapChunk×N → ChunkBatchFinished`，任何 ID 回归立即暴露
- 注册表同步是 1.20.5+ 配置阶段的强制要求，缺失会导致客户端卡在 Configuration 无法进 Play
- 出生包改用 `Login`(0x30)+`Position`(0x46) 对齐 774 客户端解析路径（旧 `SynchronizePlayerPosition`
  0x3E 在 774 已被移除/重编号）

## ADR-011：框架边界——剥离应用层世界内容

**状态**：已接受

**背景**：初版 `minestom-server::run()` 在登录时为玩家装配一个由
`create_spawn_platform` 生成的"出生平台"实例（`SharedInstance` + 草方块平台区块），
把游戏世界内容硬编码进框架/入口壳。这与"框架不提供原版游戏逻辑"的定位冲突，
也阻碍接入真实世界数据（由应用侧 `InstanceContainer` / `SpawnConfig` 提供）。

**决策**：

1. **移除应用层世界生成**：删除 `create_spawn_platform` 与 `SharedInstance` 及占位
   出生平台逻辑；`InstanceManager` 不再在框架内生成任何地形/平台内容。
2. **世界由应用侧装配**：玩家登录时的 `InstanceRef` 取自 `InstanceManager::default_instance()`；
   若应用未注册默认实例，则玩家落入 `Entity::PLACEHOLDER`（不 panic，但 `chunk_send` /
   `entity_sync` 因拿不到 `InstanceContainer` 而安全跳过，无世界可操作）。
3. **参考实现引导空默认实例**：`minestom-server::run()` 仅 `spawn(InstanceContainer::new())`
   并 `InstanceManager::set_default(entity)`——这是最小**骨架**（空容器、零地形），不是
   生成的游戏内容；真实世界由应用通过 `InstanceManager` / `SpawnConfig` 进一步装配。

**理由**：

- **内核与游戏内容分离**：框架提供协议、网络、ECS 内核与管线（chunk_send / entity_sync
  消费 `EnterPlayEvent`），游戏世界（地形、出生平台、维度）是应用职责
- **管线必须有可操作对象**：`chunk_send` 通过 `InstanceRef` 查询 `InstanceContainer` 发送
  区块批次；没有真实实例实体，区块/实体管线在真实服务器中永不运行。空默认实例让
  集成测试能验证 `ChunkBatchStart → MapChunk×N → ChunkBatchFinished` 顺序，同时不
  向框架塞入任何"世界长什么样"的假设
- **`Entity::PLACEHOLDER` 兜底保证健壮**：应用完全不配世界时服务器不崩溃，仅玩家无世界，
  符合"框架只给骨架"的最低保证
- 与 ADR-009（core/server 分工）一致：`run()` 作为入口壳负责"启动 + 装配最小世界骨架"，
  具体世界逻辑留给应用

## ADR-012：框架能力补齐的关键设计决策

**背景**：为补齐实例/区块内容、方块 API、实体框架、事件系统、碰撞物理、网络全量、
计分板/进度/配方/对话框/BossBar、调度器与注册表覆盖（`implement-framework-capabilities`），
对核心架构做以下决策。变更标识符：`[implement-framework-capabilities]`。

1. **`Section` palette 位压缩存储**：区段内部由 `Vec<u32>`（16KB/区段）改为
   「palette + 打包 u64」存储，公共 `get_block_id`/`set_block_id`/`len` 不变
   （对外零破坏）。理由：空气区段内存 <1KB、序列化可复用内部数据（消除重复打包），
   符合"工程级高性能"目标；随机读写 O(1) 平均（HashMap 定位下标 + 位运算）。
2. **`Block` 值类型 + `BlockRegistry` 扩展**：方块以 state_id 为单一事实，
   `Block` 经注册表查询 name/is_air/is_solid/属性；实心判定缺省「非空气即实心」。
   理由：协议层只传 state_id，属性/实心语义由注册数据驱动，保持值类型 `Copy` 零分配。
3. **事件系统双轨并存**：`Message` 继续承担系统间通信（固定消费点）；新增
   `EventBus`（`Listener<T>` 注册、按序派发、可取消、可解绑）供应用注册回调。
   理由：`Message` 需在 `plugin.rs` 预先注册类型且无取消语义，框架事件 API 更适合
   应用侧动态监听；两者职责不同，不合并。
4. **收件箱解耦模式**：`network_receive` 已触 `SystemParam` 16 上限，故命令
   聊天与动作类 serverbound 包经 `ClientNetworks.command_inbox` / `packet_inbox`
   写入，由独立系统（`command_chat_system` / `packet_action_system`）在
   `network_send` 前消费。理由：不删既有参数（均被下游消费）、不新增系统参数，
   复用网络队列资源零额外分配。
5. **逐轴碰撞物理**：`physics` 按 x→z→y 逐轴积分 + 方块碰撞（贴壁清轴速度），
   探测仅限实体 AABB 覆盖的方块（≤27 候选），不扫全区块。理由：无外部物理库
   依赖、行为可预期、复杂度 O(覆盖方块数)，满足"高性能"且纯函数 `move_axis`
   便于单测。
6. **网络全量补全**：serverbound 66 项 + clientbound 139 项全部实现；serverbound
   中框架关注的动作包（交互/动作/动画/使用/放置）派发事件，其余安全 Ignored；
   clientbound 全部可编码（供高层 API 使用）。0x0b 修复为权威 `client_status`。
   理由：保证流安全（任何入站包不崩）与框架 API 的协议完备性；包 ID 一律以
   vendored `PacketRegistry.java` 序位核实，不推断。
7. **调度器 BinaryHeap + 时钟源**：`TaskScheduler` 用 `BinaryHeap` 按到期 tick
   升序，`scheduler_tick` 系统以 `TickCounter` 为时钟推进。理由：O(log n) 入堆、
   无每 tick 全量扫描；延迟/周期/定点/取消四语义覆盖框架需求。
8. **简化边界（AI 裁定）**：复杂包（`DeclareCommands`/`DeclareRecipes`/
   `PlayerChatMessage` 等）线格式采用字段级简化（自洽 roundtrip、注释注明），
   `PlayerBlockPlacement` 仅派发 `BlockPlace` 事件（不写实例——放置方块类型属
   应用逻辑）；`BossBar`/`Teams` 等以枚举/字段组合表达动作。理由：框架层保证
   API 与流安全，不复制原版行为，避免过度实现。

## ADR-013：补齐 8 项部分实现的关键设计决策

**背景**：为把复杂包真实化、点击 Clone/QuickCraft、物品组件 C 档、命令参数补全、
碰撞增强、配方真实化、压缩启用、属性框架（`complete-partial-framework-capabilities`）
8 项「部分实现」补齐为完整实现，做以下决策。变更标识符：
`[complete-partial-framework-capabilities]`。

1. **复杂包真实线格式**：`DeclareCommands`（命令树图 `CommandNode`，flags 位
   0x03/0x04/0x08/0x10 条件字段，properties 按 `ArgumentParserType` 提取）、
   `DeclareRecipes`（1.21.11 新格式：`item_properties` + `stonecutter_recipes`，
   取代旧版配方列表）、`PlayerChatMessage`（完整字段 + `FilterMask` +
   `SignedMessageBodyPacked` 最小承载）全部升级为真实线格式（ADR-012 第 8 条的
   "简化边界"对这三包撤销）。理由：真实客户端可互操作；签名体系以最小承载对齐
   线格式（签名验证属应用层，不复制 Java crypto 全套）。
2. **`inventory_sync` 全量标记驱动**：`PlayerInventory.full_sync` 在登录/点击/关窗
   清空光标时置位，`inventory_sync` 仅在该标记时全量回推 WindowItems 并清零，平时
   只发脏槽增量。理由：修复"每 tick 全量回推"导致的真实 TCP 集成测试 `collect_until`
   永不空闲挂起；同时保留"点击结果随下一 tick 收敛"语义（点击置位）。
3. **点击 Clone/QuickCraft 权威建模**：Clone(3) 仅 button=0 整堆克隆（对齐 Java
   `ClickType.Middle`），非创造拒绝；QuickCraft(5) 四阶段状态机（拖起/放置全部/
   放置一半/继续）跨次点击保存拖拽快照，中途取消回滚。理由：服务端权威计算防作弊，
   拖拽状态持久化（对齐 Java `PlayerInventory.quickCraftStage`）。
4. **物品组件 C 档 + 最小文本组件**：组件 id 以 `DataComponents.java` **0 基登记序位**
   为权威（修正历史 spec 笔误：custom_data=0 而非 5）；简单自定界全量
   （Byte/Short/Long/Float/Double/String/Bool）+ NBT 承载（custom_data/enchantments）
   + 文本承载（custom_name/lore 经 `Component`↔NBT）。理由：标准线格式可与真实
   客户端互操作；`Component` 最小实现（Empty/Text/Translatable）为后续 adventure
   体系打基础；Byte/Short/Long/Double 等无对应 vanilla 顶层网络类型者映射到
   未同步登记位（roundtrip 自洽，客户端不会发送，文档注明）。
5. **碰撞 Shape + 方块形状 + 射线 + 移动求解**：`Shape`（Aabb/Empty/Merged）承载
   碰撞形态，`block_shape` 按 `is_solid` 查询（实心=单位 AABB），`RayUtils` DDA
   体素射线，`move_and_collide` 复用 `move_axis` 逐轴分解并升级实心判定。
   理由：对齐 Java `collision` 包结构但保持既有 `move_axis` API 兼容；楼梯/台阶等
   非单位形状 v1 不做（简化边界）。
6. **配方 1.21.11 真实线格式**：`RecipeProperty`/`Ingredient`/`SlotDisplay`/
   `RecipeDisplay`/`StonecutterRecipe` 值类型对齐 Java `SlotDisplayType`/
   `RecipeDisplayType` 序位；`RecipeManager::to_bytes` 输出两段式
   （item_properties + stonecutter_recipes），旧自定界格式移除（BREAKING）。
   理由：1.21.11 的 `DeclareRecipes` 已由配方列表改为 `RecipeProperty` 映射 +
   stonecutter 配方，真实客户端需要新格式。
7. **压缩启用（flate2 miniz_oxide）**：`LoginCompression`(0x03) 真实发送（默认
   阈值 256 对齐 Java，`MINESTOM_COMPRESSION_THRESHOLD` 可配，0 禁用）；帧层
   `VarInt 数据长度(0=未压缩)+payload`。`flate2` 选 `rust_backend`（miniz_oxide
   纯 Rust、无 unsafe 传递，符合章程）；**在线认证明确排除**（extras 级、依赖
   Mojang 外部服务、测试不可行），仅离线 + Velocity。理由：压缩是真实客户端联机
   刚需；在线认证超出框架核心边界。
8. **属性框架**：`Attribute` 注册数据驱动（`resources/data/attributes.toml` 35 项，
   id 对齐 Java `AttributeImpl.REGISTRY` 序位）+ `AttributeInstance::value()`
   叠加规则（ADD→ADD_MULTIPLIED_BASE（以累加后 base 乘）→MULTIPLIED_TOTAL→clamp，
   对齐 Java `computeValue`）+ `attribute_sync` 收件箱广播 `EntityAttributes`(0x81)
   （client_sync 裁剪，登录不主动下发）。理由：`EntityAttributes` 包此前仅包级
   存在，需框架级属性模型支撑；收件箱模式规避 `SystemParam` 16 上限且不破坏登录
   包序列断言。

## ADR-014：补齐 15 项缺失子系统的关键设计决策

**背景**：为补齐 adventure 文本组件、实体类层级、生物 metadata、AI 寻路、damage、
Tag<T>、snapshot、区块生成器、光照流体高度图、容器类型、item 子包、crypto、玩家
能力、potion/timer/timeline/message/ping、工具层（`complete-missing-subsystems`）
15 项缺失子系统（attribute 已在上轮 T8 完成，排除），做以下决策。变更标识符：
`[complete-missing-subsystems]`。

1. **实体类层级以组件组合表达**：Java 实体用类继承（Entity→LivingEntity→
   EntityCreature），Rust/自研 ECS 下改为组件组合——`Living`/`EntityCreature`/
   `EntityProjectile`/`ItemEntity`/`ExperienceOrb` 独立组件 + `hurt`/`navigation`
   接口。理由：符合 ECS 组合优于继承（章程禁复制翻译 Java），组件可按需挂载，
   行为经 trait/系统而非继承派发。
2. **生物 metadata 全量 + map 兜底**：`EntityMetaType` 枚举（172 变体注册表覆盖
   全部抽象层与类别）+ 手写核心 14 类 + 既有 `EntityMetadataMap` 兜底其余。
   理由：全量类型化与「核心类型 + 通用 map」平衡可维护性；map 互转桥接 0x61/0x62
   包编码（包接入留后续）。
3. **AI 经「决策层」而非 goal 直写组件**：goal 在系统中无法直接写组件，
   `Goal::update_context` + `navigation_target` 扩展方法把移动决策暴露给
   `system/entity_ai`，由系统写入 `EntityCreature.navigation_target` 再经 follower
   计算速度。理由：ECS 借用手约束下保持 goal 纯逻辑可单测。
4. **本地 crypto 签名（在线排除）**：`ed25519-dalek` 本地密钥对签名/验签；
   Mojang 在线验证与在线认证同属 extras 边界（依赖外部服务、测试不可行），排除。
   理由：框架提供签名/验签 API 与线格式承载，真实在线验证由应用/代理侧完成。
5. **Anvil 存档边界**：本变更实现 `ChunkGenerator` 接口 + `MemoryChunkLoader`
   （内存快照）；真实 MCA 区域文件读写（r./mca 解析）延后。理由：磁盘格式复杂、
   工作量大，框架先提供生成/加载接口与可工作的内存实现，MCA 另行变更。
6. **工具层不重复 std**：`utils`/`coordinate`/`thread` 仅补 Rust std 未覆盖者
   （Vec 数学/Area 迭代/时间换算/Validate），`isqrt` 委托 std、线程模型以
   std::thread + tokio 为准（`ThreadProvider` 仅接口占位）。理由：避免无意义复制
   Java 工具类。
7. **简化边界（AI 裁定）**：`LightSystem` 仅光存储 API（传播算法延后）、
   `EntitySnapshot.components` 恒空（v1 仅 position）、`Status` 描述用 plain_text、
   `Timeline` 仅位置插值（Rotation 字段保留）、`MessageSignature` 简化为
   `(salt++content)` 64 字节签名（Java 为 256 字节 SaltSignaturePair+hash）——均在
   文档注明。理由：框架保证 API 与语义对齐，不过度实现。

## ADR-015：自研 ECS 内核取代 Bevy

**状态**：已接受（取代 ADR-001 与 ADR-002 的 Bevy 选型；变更标识符：`[implement-custom-ecs]`）

**背景**：原内核基于 vendored Bevy 0.18.1（`bevy_app` / `bevy_ecs` / `bevy_time`）。在补齐 15 项子系统（`complete-missing-subsystems`）与生产加固过程中，Bevy 带来三方面阻碍：① 二进制体积与编译面庞大（即便仅取 3 个子 crate，仍拖入庞大的内部依赖树）；② 与「零外部依赖、纯标准库内核」的宪章目标冲突；③ 多实例世界并行 tick（R11 / IC-10）与跨世界实体迁移（T12.5）需要对 `World` / `Schedule` / 存储布局做深度控制，Bevy 的封闭调度模型难以契合。

**决策**：以自研 ECS 内核 `minestom-ecs`（零外部依赖，仅依赖 `std`）取代 Bevy，提供：`Entity`（分段 id：索引 + 世代，防 id 复用冲突）、`Component` + `Archetype`（静态匹配、派生宏零样板）、`SoA` 列存储 + `SparseSet`（可选组件低开销）、`Query` + `Commands`（只读/可写借用 + 延迟命令）、`Message`（系统间事件，固定消费点）、`System` + `SystemParam`（资源/查询/消息/命令的安全参数提取）、`Schedule`（系统链式 `.after()` 顺序）+ `InstanceScheduler`（多实例 `World` 并行 tick）+ `CommandQueue` + `migrate_entity`（跨 `World` 迁移）+ `Shared<T>`（跨 `World` 只读资源）。`minestom-core` 的 `App` 包裹 `World` + `Schedule`，对外 API 形态（插件装配、`Message` 收发、系统注册）与 Bevy 时代保持一致，降低迁移成本。

**理由**：

- 编译面与体积可控：仅一个纯 std crate，无图形/渲染/庞大依赖树
- 契合宪章「零外部依赖内核」「禁 unsafe（白名单）」「MSRV 1.89」硬约束
- 世界模型可深度定制：Instance 世界化（T12.5）与跨世界迁移直接改造 `World` 存储与 `InstanceScheduler` 调度，无需绕过 Bevy 内部抽象
- 性能可预期：静态 Archetype 匹配、SoA 热路径、`Shared<T>` 零拷贝跨 `World` 只读

**影响的 ADR**：ADR-001、ADR-002 标记为「已取代」；ADR-007/009/012/014 中凡以 Bevy 描述当前内核处，统一改写为自研 ECS（见各 ADR 正文）。

## ADR-016：WASM 扩展加载（wasmtime）安全评审

**状态**：已接受（变更标识符：`[complete-framework-gaps]` WS4；本 ADR 为 WS4 动工前置硬约束，未通过则 WS4 后续任务不得合入）

**背景**：为提供「运行时扩展加载」能力（填补 Java `Extension` 的运行时插件语义），
需引入 [wasmtime](https://crates.io/crates/wasmtime) 作为 WASM 运行时。wasmtime 及其
cranelift 后端含 Rust `unsafe`，属 Constitution 定义的「unsafe 传递性 crate」，须经专项
安全评审并记录后方可引入（见 Constitution「禁止的 crate 类别」）。

**决策**：wasmtime 置于 cargo feature `wasm-extensions`（**默认 off**）；启用时在
`crate::extension` 模块内以最小 `unsafe` 面完成 host FFI 注册，并满足以下五节评审要求。

### 1. unsafe 传递性说明
- wasmtime 47.x 内部大量使用 `unsafe`（cranelift 代码生成、实例内存访问等）。
- 本 crate 全局默认 `#![forbid(unsafe_code)]`；为兼容 wasmtime host 注册所需的少量
  `unsafe`，`lib.rs` 改为：
  ```rust
  #![cfg_attr(not(feature = "wasm-extensions"), forbid(unsafe_code))]
  ```
  即**默认构建零 unsafe**（合 Constitution）；仅启用 `wasm-extensions` 时降级为
  `deny(unsafe_code)`（仍禁止，仅允许在显式白名单模块内 `#[allow]`）。
- `crates/minestom-core/src/extension/` 声明为白名单模块 `#![allow(unsafe_code)]`，
  仅该模块内 host glue 的 `unsafe` 块附 `# Safety` 章节 + `debug_assert!` 护城河
  （索引边界 / 指针对齐 / 别名合法性）。
- **铁令**：涉及 `unsafe` 的 `extension/` 模块必须在 CI 中通过 `cargo miri test`
  （模拟未定义行为）。本环境（stable 工具链）下 Miri 在后台执行；白名单 unsafe 模块的
  安全性同时经 stable 测试 + clippy 覆盖。

### 2. 纯沙箱无主机内存越权
- 默认无 `wasi`、无 FS / 网络 capability；host 导入白名单仅含回调 / 事件类函数
  （`host_register_tick_callback` / `host_register_event`），**变更型 host API**
  （`host_set_block` / `host_spawn_entity` / `host_register_system`）按签名预留
  `#[cfg(feature = "extension-mutation")]` 占位、v1 默认关闭、不接线。
- WASM 实例**永远经 host 调用**，绝不直接触碰 ECS 存储（最小权限 + 边界清晰）。
- v1 能力定位为「只读观察者 + 回调」：扩展仅能注册 tick 回调与事件监听，不得自行
  spawn 实体 / 改方块（与 Java `Extension` 深度改逻辑不对等，属已知弱化）。

### 3. wasmtime 版本精确锁定
- `wasmtime = "=47.0.3"`（精确锁定，纳入 `Cargo.lock`，禁止浮动解析）。
- **MSRV 偏离说明**：wasmtime 47.0.3 要求 Rust ≥ **1.94**，而本仓库 Constitution 声明
  MSRV 1.89。该偏离**仅影响 `wasm-extensions` feature 矩阵**：默认构建（feature off）
  仍维持 1.89；CI 须为 `wasm-extensions` 矩阵使用 Rust ≥1.94 的独立 job，不污染默认矩阵。
  本机工具链为 1.97.1，可同时覆盖两条矩阵。

### 4. CI 含 `cargo audit`
- 默认矩阵与 `wasm-extensions` 矩阵**均**跑 `cargo audit`（RUSTSEC 检查）。
- wasmtime 引入的传递依赖（cranelift 系列等）若触发 RUSTSEC 高危，须升级或加审计豁免
  并说明理由，不得静默忽略。

### 5. 沙箱能力边界 + 编译影响 + tick 预算 + api=0 取舍
- **能力边界（最小权限）**：host 导入白名单 + 无 wasi/FS/网络；变更型 API 默认关闭。
- **编译影响**：cranelift 重量级，编译面显著增大。CI 拆双矩阵分别运行：默认矩阵
  不拉取 wasmtime（构建快），`wasm-extensions` 矩阵单独构建。
- **tick 性能预算**：每 tick 派发所有扩展回调；单次 wasmtime 调用 tens~hundreds ns，
  20Hz（50ms）预算宽松，但多扩展累积——限定 `MAX_EXTENSIONS = 64`；预留批量派发入口
  （宿主每 tick 调用一次 `minestom_tick`，扩展内部分发）以摊薄边界切换开销；补
  `benches/extension_tick.rs` 基准骨架测量单机 tick 成本。
- **`minestom_init(api=0)` 取舍**：扩展导出 `extern "C" fn minestom_init(api: i32) -> i32`，
  `api` 参数**刻意保留为 0 占位**（本实现简化为直接调用 host 导入，不传 vtable 指针）。
  迁移路径：当 host 函数数量膨胀或需版本协商时，切为传入 vtable 指针（api 即 vtable
  地址），届时仅需升级 `minestom_init` 调用约定，扩展侧可平滑迁移。

## ADR-017：在线 Mojang 认证依赖审计（online-auth）

**状态**：已接受（变更标识符：`[complete-framework-gaps]` WS5b；feature `online-auth` 默认 off）

**背景**：在线认证（加密握手 + hasJoined 验证）需 RSA 加密、AES 对称加密与 TLS HTTP
客户端。这些依赖须为**纯 Rust、安全、无 unsafe 传递性**（Constitution 硬约束），且
默认 off 不污染离线构建。

**决策**：以下依赖均经纯 Rust 安全 crate 审计，置于 feature `online-auth`（默认 off）：

| 依赖 | 精确版本 | 性质 | 安全结论 |
| --- | --- | --- | --- |
| `rsa` | `=0.9.10` | 纯 Rust RSA（RustCrypto） | 纯 Rust、无 unsafe 传递；已知 RUSTSEC-2023-0071（Marvin 时序侧信道，仅影响网络可达的私钥解密路径，本场景仅用于服务端验签/加密握手，风险可控，记录待办） |
| `aes` | `=0.9.2` | 纯 Rust AES（RustCrypto） | 纯 Rust、无 unsafe 传递；用于 AES-CFB8 流加密 |
| `rustls` | `=0.23.43` | 纯 Rust TLS | 纯 Rust；底层依赖 `ring`（含 C/asm，属unsafe传递）——**偏离点**：`ring` 为业界广泛审计的纯 Rust 生态标准 TLS 后端，本 ADR 记录其作为 rustls 默认后端的接受理由；若需零 unsafe 传递，可后续切 `aws-lc-rs`（仍含 C）。为最小化默认构建影响，`online-auth` 矩阵才拉取 rustls |
| `rustls-native-certs` / `webpki-roots` | 精确锁定 | 信任根 | 纯 Rust 数据，无 unsafe |

**版本锁定与审计**：所有依赖精确锁定并纳入 `Cargo.lock`；`online-auth` 矩阵跑 `cargo audit`。
**默认零副作用**：feature off 时不引入上述任何依赖、无网络出口、无 RSA/AES 代码编译，
离线登录语义与现状完全一致（见 ADR-014 第 4 条「在线认证排除」的推翻，仅以默认 off 的
feature 形式提供，不违 Constitution）。
