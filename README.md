<!-- Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
SPDX-License-Identifier: GPL-3.0-or-later -->
# ParticleMCFramework

基于高性能网络编程的 Minecraft 服务器框架，使用 **Rust 2024 + 自研 ECS（particlemc-framework-ecs）** 构建。当前交付为**可用框架层**：真实 TCP 监听、1.21.11 协议编解码、完整的 `Handshake → Status → Login → Configuration → Play` 状态机、玩家出生与移动、原生 Velocity modern forwarding 转发校验，以及三层发包模型。

> 本框架**不提供原版游戏逻辑**，而是提供高性能、可扩展的服务端内核，不推荐直接作为服务端核心。游戏内容以「注册数据 + 事件系统」组织。

***

## 目录

- [ParticleMCFramework](#particlemcframework)
  - [目录](#目录)
  - [特性](#特性)
  - [快速开始](#快速开始)
    - [环境变量](#环境变量)
  - [项目结构](#项目结构)
    - [Crate 分工](#crate-分工)
  - [技术细节](#技术细节)
    - [架构总览](#架构总览)
    - [20Hz tick 管线](#20hz-tick-管线)
    - [协议层](#协议层)
    - [连接状态机](#连接状态机)
    - [三层发包模型](#三层发包模型)
    - [网络桥接：tokio ↔ 游戏主循环](#网络桥接tokio--游戏主循环)
    - [Velocity modern forwarding](#velocity-modern-forwarding)
    - [注册表与数据层](#注册表与数据层)
    - [实例与世界模型](#实例与世界模型)
    - [事件系统](#事件系统)
  - [1.21.11 登录流程详解](#12111-登录流程详解)
  - [依赖与版本锁定](#依赖与版本锁定)
  - [项目约束与代码规范](#项目约束与代码规范)
  - [测试](#测试)
  - [已知限制与后续路线](#已知限制与后续路线)
  - [相关文档](#相关文档)

***

## 特性

- **真实 TCP 监听**：`tokio` 异步绑定端口、按连接分配 `conn_id`，读写任务与游戏循环解耦。
- **1.21.11 协议（protocol 774）**：Handshake / Status / Login / Configuration / Play 五阶段的最小真实数据包编解码（VarInt 帧格式、大端字段、Position 位打包），协议 ID 已按 774 校准。
- **完整状态机**：`ConnectionState` 枚举 + 合法转换校验；监听侧与游戏侧双状态表协同推进。
- **玩家出生与移动**：登录成功即生成玩家实体（`Player` / `Position` / `Health` / `Velocity` / `InstanceRef` 组件），`PlayerPosition` / `PlayerPositionAndRotation` / `Look` / `Flying` 派生为 `PlayerMoveRaw` 事件。
- **Velocity modern forwarding**：握手包末位容错读取转发 blob，HMAC-SHA256 签名校验（常量时间比较）+ 时间戳防重放（±5s），强制代理开关。
- **三层发包模型**：`urgent` 立即送达 / `normal` 按 MTU 聚合 / `ChunkSender` 信用节流，广播一次序列化多处复制。
- **配置阶段注册表同步**：`RegistryData` ×N + `UpdateTags` + `FeatureFlags` 按 774 顺序下发，NBT 网络编码（`encode_root` / `encode_anonymous`）。
- **区块批次管线**：玩家进入 Play 后按 `EnterPlayEvent` 发送 `ChunkBatchStart → (MapChunk)×N → ChunkBatchFinished`，`ChunkSender` 按客户端 `ChunkBatchReceived` 回包推进信用。
- **实体同步**：多玩家互见广播（`SpawnEntity` + `PlayerInfo`），玩家上下线派生 `PlayerJoin` / `PlayerQuit`。
- **20Hz tick 管线**：自研 `Schedule` 固定步长，9 个系统链式依赖确定执行顺序。
- **数据驱动注册表**：方块 / 物品 / 实体类型 / 生物群系等从 `resources/data/*.toml` 加载，`serde` 反序列化。
- **零 unsafe**：`#![forbid(unsafe_code)]`，生产代码禁止 `unwrap()` / `expect()`。

***

## 快速开始

前置要求：Rust 1.89+（edition 2024），无外部 ECS 依赖（纯标准库内核），构建无需网络。

```bash
# 构建整个 workspace
cargo build --workspace

# 运行全部单元测试 + 集成测试（含真实 TCP 伪客户端登录测试）
cargo test --workspace

# 严格 clippy 门禁（生产代码禁用 unwrap/expect）
cargo clippy --workspace -- -D warnings -D clippy::unwrap_used -D clippy::expect_used

# 启动服务器（默认监听 0.0.0.0:25565）
cargo run -p particlemc-framework-server
```

### 环境变量

| 变量                                     | 说明                                     | 默认值             |
| -------------------------------------- | -------------------------------------- | --------------- |
| `PARTICLE_MCFRAMEWORK_BIND_ADDR`       | 监听地址（`SocketAddr`，如 `127.0.0.1:25566`） | `0.0.0.0:25565` |
| `PARTICLE_MCFRAMEWORK_VELOCITY_SECRET` | Velocity 转发共享密钥；非空即启用转发校验              | 未设置（直连模式）       |

Velocity 转发也可通过 `config/velocity.toml` 配置：

```toml
secret = "your-forwarding-secret"
enforce = true   # true 时拒绝无有效转发的连接
```

***

## 项目结构

```
rust/ParticleMCFramework/
├── Cargo.toml                  # workspace 根（resolver=2，全部依赖精确锁定）
├── Cargo.lock
├── CHANGELOG.md
├── docs/
│   └── decisions.md            # 架构决策记录（ADR）
├── config/
│   └── velocity.toml           # Velocity 转发配置（可选）
├── resources/
│   └── data/                   # 注册数据（blocks/items/entity_types/biomes... 的 TOML）
├── tools/                      # 数据转换工具（保持原样，不被本 workspace 改动）
└── crates/
    ├── particlemc-framework-core/   # 核心框架库（服务器内核、数据层、插件接口）
    │   ├── src/
    │   │   ├── lib.rs          # crate 根：模块声明 + 项目约束
    │   │   ├── plugin.rs       # McServerPlugin：装配资源/事件/系统
    │   │   ├── schedule.rs     # 20Hz 固定步长配置
    │   │   ├── component/      # ECS 组件（Position/Velocity/Health/Player/InstanceRef/BlockState）
    │   │   ├── event/          # 自研 `Message` 事件（PlayerJoin/PlayerQuit/NetworkEvent...）
    │   │   ├── network/        # 网络层
    │   │   │   ├── listener.rs # 真实 TCP 监听 + 帧解析 + 状态标签
    │   │   │   ├── bridge.rs   # NetworkBridge：tokio 监听 ↔ 游戏主循环
    │   │   │   ├── client.rs   # ClientNetwork 三层发包模型 + ChunkSender
    │   │   │   ├── connection.rs # ConnectionState 状态机
    │   │   │   └── packet_codec.rs # 编解码 trait 骨架（遗留占位，未接入实时管线；实时编解码见 protocol/packet.rs 的 Packet trait）
    │   │   ├── protocol/       # 协议层
    │   │   │   ├── framing.rs  # VarInt 长度帧封装（MAX_FRAME=2MiB）
    │   │   │   ├── varint.rs   # VarInt / VarLong
    │   │   │   ├── byte_buf.rs # 游标式字节缓冲（大端读写）
    │   │   │   ├── error.rs    # ProtocolError
    │   │   │   ├── packet.rs   # Packet trait
    │   │   │   ├── dispatch.rs # (state, packet_id) → InboundPacket 派发表
    │   │   │   ├── velocity.rs # Velocity modern forwarding 校验
    │   │   │   └── packets/    # 各阶段最小真实数据包
    │   │   ├── resource/       # Manager 类 Resource + 注册表
    │   │   │   ├── connection_manager.rs / instance_manager.rs / command_manager.rs / scheduler_manager.rs
    │   │   │   ├── velocity_config.rs
    │   │   │   └── registries/ # Block/Item/EntityType/Biome/... 注册表
    │   │   ├── instance/       # 世界模型（Chunk/Section/InstanceContainer）
    │   │   └── system/         # 20 TPS 管线 9 系统
    │   └── tests/
    │       └── registry_data_integration.rs  # 注册数据加载集成测试
    └── particlemc-framework-server/  # 服务器二进制入口
        ├── src/
        │   ├── lib.rs          # run()：装配 App + tokio 运行时 + 20Hz 主循环
        │   └── main.rs         # 二进制入口（banner + 启动）
        └── tests/
            └── fake_client_login.rs  # 真实 TCP 伪客户端登录集成测试
```

### Crate 分工

下表列出本 workspace 中各 crate 的分工，**分工不同而非两个独立项目**。

| crate                                | 角色              | 内容                                                                                                                                                                   | 对外暴露                                          |
| ------------------------------------ | --------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------- |
| `crates/particlemc-framework-core`   | **框架库**（全部业务逻辑） | 自研 ECS（particlemc-framework-ecs）内核、协议层、网络层、注册表、系统管线；公共模块 `component` / `event` / `instance` / `network` / `plugin` / `protocol` / `resource` / `schedule` / `system` | 仅以 Rust 库形式被链接（`[lib] crate-type = ["rlib"]`） |
| `crates/particlemc-framework-server` | **入口二进制壳**      | `main.rs` 读取环境变量并启动；`lib.rs` 的 `run()` 装配 `App` + tokio 运行时 + 20Hz 主循环，供二进制与 `tests/fake_client_login.rs` 集成测试复用                                                     | 可执行入口 + 集成测试宿主                                |

真正的服务器逻辑全部在 `particlemc-framework-core` 库中，`particlemc-framework-server` 只负责"启动"这件事。

***

## 技术细节

### 架构总览

服务器内核基于**自研 ECS 内核** **`particlemc-framework-ecs`**（零外部依赖，仅依赖标准库）：

- `App` 是服务器本体；`McServerPlugin` 负责把全部 `Resource`、注册表、`Message`（事件）与 9 个 tick 系统装配进 `App`。
- 游戏循环由 `particlemc-framework-server::run` 驱动：主线程以 `app.update() + 50ms sleep` 构成 20Hz 主循环；`tokio` 多线程运行时承载异步 TCP 监听与读写任务。
- 网络层与游戏层通过 `NetworkBridge` 解耦：监听任务推入站帧、`network_receive` 消费；`network_send` 把出站队列刷给监听侧写任务。

```
┌─────────────────────────── 主线程（游戏同步循环）──────────────────────────┐
│  App::update() 每 50ms
│   └─ 主循环驱动 20Hz 固定步长 `Schedule`
│        ├─ network_receive  ← 消费 NetworkBridge.inbound（tokio mpsc Receiver）
│        ├─ ... 游戏逻辑系统 ...
│        └─ network_send     → flush 到 NetworkBridge.outbound（每连接 Sender）
└──────────────────────────────────┬────────────────────────────────────────┘
                                   │ 入站帧 / 出站字节
┌──────────────────────────────────▼────────────────────────────────────────┐
│  tokio 多线程运行时（ConnectionListener）                                   │
│   ├─ accept 循环：每连接分配 conn_id，注册出站通道，派生读写任务            │
│   ├─ 读任务：按 Minecraft 帧格式（VarInt 长度 + packet_id + body）解析，     │
│   │   打状态标签后推入 inbound 通道                                        │
│   └─ 写任务：从本连接出站通道取字节，write+flush 回 socket                  │
└────────────────────────────────────────────────────────────────────────────┘
```

### 20Hz tick 管线

自研固定步长机制：主循环以 50ms 步长驱动 20Hz 固定步长 `Schedule`；`configure_20hz` 将步长覆写为 20Hz（50ms），匹配 Minecraft 的 tick 频率。

9 个系统按链式 `.after()` 在固定步长 `Schedule` 中固定顺序：

```
network_receive → tick_begin → player_input → player_movement → entity_ai
      → physics → chunk_dirty_sync → tick_end → network_send
```

- **network\_receive**：消费入站帧，按协议状态机推进连接、生成/移除玩家实体、派生移动事件。
- **tick\_begin / tick\_end**：tick 首尾钩子（tick 计数等）。
- **player\_input**：消费 `NetworkEvent::PlayerMoveRaw` 等，落到玩家实体。
- **player\_movement / entity\_ai / physics**：移动同步、实体 AI（goal 选择 + 寻路移动）、基础物理。
- **chunk\_dirty\_sync**：脏区块同步标记。
- **network\_send**：flush 全部在线玩家的出站队列并清空（含 urgent 紧急窗口递减）。

### 协议层

**帧封装**（`protocol/framing.rs`）：每个数据包在线上为 `VarInt 长度 + payload`，`payload` 内含 `packet_id`（VarInt）与包体。`MAX_FRAME = 2 MiB` 与 Minecraft 默认上限一致，超限报 `FrameTooLarge`。

**VarInt / VarLong**（`protocol/varint.rs`）：7 位分组 + 最高位续延标志，最长 5 字节（VarInt）/ 10 字节（VarLong），越界报 `VarIntTooLong`。

**ByteBuffer**（`protocol/byte_buf.rs`）：游标式读写缓冲，所有整数按大端（网络字节序）；越界统一返回 `ProtocolError::UnexpectedEof`。方块坐标按 26/12/38 位打包/解包。

**Packet trait**（`protocol/packet.rs`）：`packet_id()` + `encode(&mut ByteBuffer)` + `decode(&mut ByteBuffer)`；`encode_clientbound` 将出站包编码为 `packet_id + body`。

**派发表**（`protocol/dispatch.rs`）：`(ConnectionState, packet_id) → InboundPacket`。握手/状态/登录/配置阶段未知 id 报 `UnknownPacket`（记录后跳过、不 panic）；游玩阶段大量未处理包静默 `Ignored`。

**已实现的数据包（1.21.11 / protocol 774 校准）**：

| 状态            | 入站（serverbound）                                                                                                                                                                                                                            | 出站（clientbound）                                                                                                                                                                                                                                                      |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Handshake     | `Intention` (0x00)                                                                                                                                                                                                                         | —                                                                                                                                                                                                                                                                    |
| Status        | `StatusRequest` (0x00)、`Ping` (0x01)                                                                                                                                                                                                       | `StatusResponse`、`PingResponse`                                                                                                                                                                                                                                      |
| Login         | `Hello` (0x00)、`LoginAcknowledged` (0x03)                                                                                                                                                                                                  | `LoginSuccess` (0x02)、`LoginDisconnect`、`LoginCompression`                                                                                                                                                                                                           |
| Configuration | `ClientInformation` (0x00)、`FinishConfiguration` (0x03)                                                                                                                                                                                    | `RegistryData` (0x07)、`UpdateTags` (0x0d)、`FeatureFlags` (0x0c)、`FinishConfigurationS2C` (0x03)、`ConfigDisconnect` (0x02)、`PluginMessage`                                                                                                                            |
| Play          | `PlayerPosition` (0x1d)、`PlayerPositionAndRotation` (0x1e)、`Look` (0x1f)、`Flying` (0x20)、`KeepAlive` (0x1b)、`TeleportConfirm` (0x00)、`ClientCommand` (0x0b)、`PlayerLoaded` (0x2b)、`ChunkBatchReceived` (0x0a)、`FinishConfiguration` (0x03) | `Login` (0x30)、`Position` (0x46)、`UpdateHealth` (0x66)、`PlayerInfo` (0x44)、`SpawnEntity` (0x01)、`MapChunk` (0x2c)、`ChunkBatchStart` (0x0c)、`ChunkBatchFinished` (0x0b)、`EntityTeleport` (0x7b)、`RelEntityMove` (0x33)、`PlayerRemove` (0x43)、`GameStateChange` (0x26) |

> 1.21.11（protocol 774）关键 ID 校准与历史偏差见 `docs/decisions.md` 的 ADR-010；框架边界决策见 ADR-011。

### 连接状态机

`ConnectionState`（`network/connection.rs`）描述五阶段流转，`transition()` 仅在合法转换上返回 `Ok`：

```
Handshake ──(Intention.next_state=1)──► Status ──(StatusRequest/Ping)──►（可响应后关闭）
     │
     └──(next_state=2)──► Login ──(Hello→LoginSuccess→LoginAcknowledged)──► Configuration
                                                                                │
                                                                                └──(ClientInformation→FinishConfiguration S2C→FinishConfiguration C2S)──► Play
```

**双状态表设计**：监听侧（`listener.rs` 的 `conn_states`）只维护「为入站帧打状态标签」所需的最小状态，随收到的包推进（Handshake 帧解码 `Intention` 后置为 Status/Login；Login 0x03 → Configuration；Configuration 0x03 → Play）；游戏侧 `ConnectionManager` 维护权威状态，供登录/身份/实体绑定使用。两侧以 `conn_id` 关联。

### 三层发包模型

`ClientNetwork`（`network/client.rs`）是每连接的单发送状态，按优先级分三层：

1. **urgent 队列**：登录 / 传送 / 伤害 / 聊天等必须立即送达的包，tick 末逐包 `write + flush`。
2. **normal 队列**：实体移动 / 区块数据 / 粒子等可容忍 ≤50ms 延迟的包，累积进 `write_buffer`，达到 `mtu_threshold`（默认 1400）或队列空时一次性 `write + flush`。
3. **ChunkSender**：信用节流器，初始信用 9.0，按客户端 `ChunkBatchReceived` 回包动态调整区块发送速率（每 tick 累加、上限 16/tick）。

配套机制：`broadcast` 对 N 个玩家只序列化一次、字节复制到各目标队列；`urgent_window_ticks` 子服切换紧急窗口内普通包按紧急处理。

### 网络桥接：tokio ↔ 游戏主循环

`NetworkBridge`（`network/bridge.rs`）持有：

- `inbound: tokio::sync::mpsc::Receiver<RawFrame>`：监听任务 → 游戏循环的入站帧通道（容量 1024）。
- `outbound: Arc<Mutex<HashMap<u32, Sender<Vec<u8>>>>>`：游戏循环 → 监听任务的出站通道表（每连接一个容量 256 的通道）。

`RawFrame` 两种载荷：`Packet { conn_id, state, packet_id, payload }` 与 `Closed(conn_id)`。同步侧用 `try_recv` / `try_send` 非阻塞存取，异步侧用 `recv().await` / `send().await`，两侧互不阻塞。

### Velocity modern forwarding

`protocol/velocity.rs` 实现 1.20.3+ 的现代转发校验（`hmac` + `sha2`，纯 Rust、无 unsafe）：

- 经 Velocity 代理转发的玩家，其握手包末尾追加 blob：`version(u8=1) + signature([u8;32]) + payload`。
- `payload = uuid(16) + name(String) + timestamp(i64) + properties(Vec)`。
- 校验：用共享密钥重算 `HMAC-SHA256(secret, version ++ payload)`，与签名做**常量时间比较**（防时序攻击）；校验 `timestamp` 与当前时间差 ≤ `MAX_SKEW`（5000ms，防重放）。
- `VelocityConfig`：`secret` 为空则不校验（直连）；`enforce_proxy=true` 时无有效转发的连接直接拒绝。
- 身份经 `ForwardedIdentity` 落到连接，登录时优先使用（取代客户端自报 UUID）。

### 注册表与数据层

`resource/registries/` 提供从 TOML 加载的注册表：`BlockRegistry` / `ItemRegistry` / `EntityTypeRegistry` / `BiomeRegistry` / `DimensionTypeRegistry` / `FluidRegistry` / `ParticleRegistry` / `SoundEventRegistry` / `DamageTypeRegistry` / `EnchantmentRegistry` / `PotionEffectRegistry` / `TagRegistry` / `GenericRegistry` / `LootTableRegistry`。

- 数据源：`resources/data/*.toml`、`resources/data/tags/`、`resources/data/generic/`、`resources/data/loot_tables/`（共 72 个文件，全部入库）。
- 加载：`toml`（仅 `parse` 特性）+ `serde` 反序列化；加载失败时回退默认空注册表（`unwrap_or_default`），不 panic。
- **GenericRegistry**（`generic/`，44 文件 969 条 `[[entry]]`，合并后唯一键 923）：通用变体类注册数据以 `HashMap<String, toml::Value>` 原样存盘；无 `name` 条目回退以 `id` 十进制字符串为键（如 `sound_sources` 的 `"0"`），保证零丢失。
- **LootTableRegistry**（`loot_tables/`，4 文件 1275 条）：轻量 `LootTable { name, table_type, random_sequence, pools, raw }`——`name/type/random_sequence` 强类型便于查询，`pools` 保留原始 `toml::Value`（嵌套结构不反序列化、不丢失），`raw` 恒为整条 entry 保真（含顶层附加字段）。
- 策略：`McServerPlugin::new()` 加载核心注册表（方块/物品/实体类型）+ **generic/loot（始终加载）**；`with_preload()` 全量预热（追加世界类注册表与标签）。

### 实例与世界模型

`instance/` 提供 Minecraft 服务器世界模型的核心抽象：

- `Section`：16×16×16 区段（`SECTION_VOLUME = 4096`）。
- `Chunk`：16×16 区块，含 24 个区段。
- `InstanceContainer`：方块读写的最小真实逻辑容器（组件，可被 `chunk_send` 查询）。
- `InstanceManager`：默认实例 + 已注册实例表。**框架不生成任何地形内容**（出生平台等），世界由应用侧通过 `InstanceManager` / `SpawnConfig` 装配；`particlemc-framework-server` 的 `run()` 仅引导一个空 `InstanceContainer` 作为默认实例，使 `chunk_send` / `entity_sync` 有可操作对象。

### 事件系统

自研 ECS 将事件类型命名为 **`Message`**：`#[derive(Message)]` 定义、`App::add_message` 注册、`MessageWriter::write` / `MessageReader::read` 收发。

已注册事件：`BlockBreak` / `BlockPlace` / `PlayerJoin` / `PlayerQuit` / `PlayerMove` / `EntityDamage` / `EntityDeath` / `NetworkEvent` / `PacketSendEvent`。

***

## 1.21.11 登录流程详解

以离线（直连）流程为例，伪客户端集成测试（`fake_client_login.rs`）完整覆盖：

| 步骤 | 客户端 → 服务端                                                         | 服务端 → 客户端                                                                                  | 说明                                                                           |
| -- | ----------------------------------------------------------------- | ------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------- |
| 1  | `Intention` (Handshake 0x00, protocol=774, next\_state=2)         | —                                                                                          | 监听侧解码 `Intention` 置状态为 Login；游戏侧登记连接                                         |
| 2  | `Hello` (Login 0x00: name + has\_uuid + uuid)                     | —                                                                                          | 生成玩家实体（`Player`/`Position`/`Health`/`Velocity`/`InstanceRef`），发 `PlayerJoin` |
| 3  | —                                                                 | `LoginSuccess` (Login 0x02: uuid + name + properties)                                      | 身份：Velocity 转发优先，否则离线 UUID                                                   |
| 4  | `LoginAcknowledged` (Login 0x03)                                  | —                                                                                          | 连接进入 Configuration                                                           |
| 5  | `ClientInformation` (Config 0x00)                                 | —                                                                                          | 触发服务端下发配置阶段注册表同步                                                             |
| 6  | —                                                                 | `RegistryData` (0x07)×N → `UpdateTags` (0x0d) → `FeatureFlags` (0x0c)                      | 配置阶段注册表同步（NBT 网络编码），顺序固定                                                     |
| 7  | —                                                                 | `FinishConfigurationS2C` (Config 0x03)                                                     | 告知客户端配置完成                                                                    |
| 8  | `FinishConfiguration` (Config 0x03)                               | —                                                                                          | 连接进入 Play                                                                    |
| 9  | —                                                                 | `Login` (Play 0x30) + `Position` (0x46, 出生点) + `UpdateHealth` (0x66) + `PlayerInfo` (0x44) | 出生（urgent 立即送达）；随后写入 `EnterPlayEvent`                                        |
| 10 | —                                                                 | `ChunkBatchStart` (0x0c) → `(MapChunk 0x2c)×N` → `ChunkBatchFinished` (0x0b)               | 出生区块批次（`chunk_send` 消费 `EnterPlayEvent`）；空实例为 0 个区块的零批次                      |
| 11 | `ChunkBatchReceived` (Play 0x0a)                                  | —                                                                                          | 推进该连接 `ChunkSender` 信用（加速后续批次）                                               |
| 12 | `PlayerPosition` (Play 0x1d) / `PlayerPositionAndRotation` (0x1e) | —                                                                                          | 派生 `PlayerMoveRaw`，验证连接保持                                                    |

Velocity 转发流程差异仅在第 1 步：握手包末尾携带 blob，`verify_forwarding` 校验通过后，第 2 步身份直接使用代理提供的真实 UUID/名字/皮肤属性。

***

## 依赖与版本锁定

- **particlemc-framework-ecs**：自研 ECS 内核（零外部依赖，纯标准库），提供 `World` / `Schedule` / `System` / `Component` / `Resource` / `Message` / `Shared`，经本地 `path` 依赖引用。
- **tokio 1.44.0**：`=1.44.0`，启用 `rt` / `rt-multi-thread` / `net` / `io-util` / `time` / `sync`。
- **serde 1.0.229 / toml 0.8.20 / uuid 1.13.2 / hmac 0.12.1 / sha2 0.10.8**：全部精确锁定；`toml` 仅启用 `parse` 特性（规避与 serde 1.0.229 的 display 特性 trait 导入冲突）。
- **MSRV**：1.89（edition 2024）。
- 传递依赖由 `Cargo.lock` 固化（应用项目保留 lockfile）。

## 项目约束与代码规范

- `#![forbid(unsafe_code)]`：禁止不安全代码。
- 生产代码禁止 `unwrap()` / `expect()`（`-D clippy::unwrap_used -D clippy::expect_used`）；仅测试代码通过 `#![cfg_attr(test, allow(...))]` 豁免。
- 生产代码禁止裸 `[i]` 索引、禁止 `as` 缩窄转换（相关 clippy lint 计入门禁）。
- 全部依赖精确版本锁定（`=x.y.z`）。
- 文档与注释使用中文。

## 测试

- **单元测试**（内嵌 `#[cfg(test)]`）：状态机转换、ByteBuffer 往返、帧封装边界、VarInt 边界、Velocity 转发（有效/密钥错误/过期/畸形）、发包模型（urgent/normal/broadcast/ChunkSender 信用/紧急窗口）、注册表加载、插件装配冒烟等。
- **集成测试**：
  - `particlemc-framework-core/tests/registry_data_integration.rs`：真实 `resources/data` 注册数据加载验证。
  - `particlemc-framework-server/tests/fake_client_login.rs`：真实 TCP 连入，按 1.21.11（protocol 774）离线流程完成登录并进入游玩，断言配置/游玩阶段包序列与顺序（RegistryData×N → UpdateTags → FeatureFlags → FinishConfiguration；Login → Position → UpdateHealth → PlayerInfo；ChunkBatchStart → MapChunk×N → ChunkBatchFinished），回 `ChunkBatchReceived` 后连接保持；另含双玩家互见（`SpawnEntity` + `PlayerInfo`）测试与**压缩开关**测试（threshold=256 断言收到 `LoginCompression` 且后续帧可解压；threshold=0 帧格式原样）。
  - `particlemc-framework-server/tests/velocity_forwarding.rs`：Velocity modern forwarding 校验。

## 已知限制与后续路线

- 登录为**离线模式**（无 Mojang/Yggdrasil 在线认证）；Velocity 转发提供代理场景下的真实身份。**在线认证明确不在框架层范围**（依赖 Mojang 外部服务；需要在线认证请置于应用层/代理）。**crypto 签名为本地 Ed25519 签名/验签**，Mojang 在线消息验证同样排除（依赖外部服务）。
- **压缩已启用**：登录后发送 `LoginCompression`（默认阈值 256，`PARTICLE_MCFRAMEWORK_COMPRESSION_THRESHOLD` 可配，0 禁用），帧层支持 zlib 压缩/解压（`flate2` miniz\_oxide 纯 Rust）。
- 区块数据发送已具备 `ChunkBatchStart` / `MapChunk` / `ChunkBatchFinished` 包与 `ChunkSender` 信用节流；**区块生成/存档**：`ChunkGenerator` 接口 + `MemoryChunkLoader` 内存快照已实现，**真实 MCA 区域文件读写（Anvil 格式）延后**（另行变更）。
- 实体 AI / 物理：碰撞已具备 `Shape` + 方块形状 + 体素射线 + 逐轴移动；*AI 已具备 GoalSelector/TargetSelector + 7 goal + 2 target + A* 寻路\*（`Navigator`/4 生成器/4 follower），`system/entity_ai` 每 tick 执行 goal 选择与移动；楼梯/台阶等非单位方块形状 v1 不做。
- 配置阶段注册表同步（`RegistryData` ×N + `UpdateTags` + `FeatureFlags`，NBT 网络编码）已实现；注册表数据来自 `resources/data/*.toml`，尚未覆盖全部原版注册表条目（属性 `attributes.toml` 35 项、伤害类型 50 项已覆盖常见项）。
- 物品组件 C 档：简单自定界组件全量 + `custom_name`/`lore` 文本承载 + NBT 承载（`custom_data`/`enchantments` 等）；`Byte`/`Short`/`Long`/`Double` 等无 vanilla 顶层网络类型的组件映射到未同步登记位（roundtrip 自洽，客户端不会发送）；未知组件 id 解码返回 `UnsupportedComponents`。附魔/书承载与 `ItemHandler` 行为 API 已提供（`ItemStack` 保持值类型语义，handler 经独立注册表挂载）。
- `PlayerChatMessage` 签名体系为最小承载（对齐线格式，`SignedMessageBodyPacked` 不含 last\_seen；签名验证属应用层——crypto 模块提供本地签名/验签 API）。
- **简化边界（v1，均文档注明）**：`LightSystem` 仅光存储 API（无传播算法）；`EntitySnapshot` 仅捕获 position（components 恒空）；`Timeline` 仅位置插值；`MessageSignature` 为 `(salt++content)` 64 字节签名（非 Java 256 字节 SaltSignaturePair+hash）；`Status` MOTD 描述用 plain\_text。

## 相关文档

- `docs/decisions.md`：架构决策记录（ADR-001 ~ ADR-015）。
- `CHANGELOG.md`：变更日志。
- `crates/particlemc-framework-core/src/lib.rs`：crate 级模块说明。
