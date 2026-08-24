// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
// 全局 unsafe 禁令：默认构建零 unsafe（合 Constitution）。
// 仅当启用 `wasm-extensions`（引入 wasmtime host FFI 的少量 unsafe）时降级为
// `deny`——仍禁止 unsafe，仅在 `extension/` 白名单模块内以 `#[allow]` 显式开放。
// 见 `docs/decisions.md` ADR-016（wasmtime 安全评审）。
#![cfg_attr(not(feature = "wasm-extensions"), deny(unsafe_code))]
#![cfg_attr(feature = "wasm-extensions", deny(unsafe_code))]
// 测试代码允许使用 unwrap/expect 和 unsafe（env::set_var 在 Rust 2024 中是 unsafe）
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
#![cfg_attr(test, allow(unsafe_code))]
//! # particlemc-framework-core
//!
//! Minestom（Rust 重写版）的核心框架库。
//!
//! 本项目以 [Minestom](https://github.com/Minestom/Minestom) 为设计蓝本，
//! 使用 Rust + 自研 [particlemc-framework-ecs] 内核重写服务端框架，目标是在不牺牲性能的前提
//! 下提供模块化、可组合的 Minecraft 服务器框架（particlemc-framework-ecs 替代旧 ECS 方案，RM1）。
//!
//! 当前交付为**可用框架层**：真实 TCP 监听、1.21.11 协议编解码、完整的
//! Handshake → Status → Login → Configuration → Play 状态机、玩家出生与移动，
//! 以及原生 Velocity modern forwarding 转发校验。`cargo build`、单元测试与
//! 伪客户端集成测试均可通过。
//!
//! ## 模块划分
//!
//! - [`plugin`]：`McServerPlugin` 等自建 App 插件入口，负责装配服务器数据与系统
//! - [`schedule`]：20Hz 固定步长（20 TPS）配置
//! - [`resource`]：Manager 类 `Resource` 与各类注册表（方块/物品/实体类型……）
//! - [`component`]：实体数据组件（坐标/速度/生命值/玩家/实例引用/方块状态）
//! - [`system`]：20 TPS tick 管线的系统（network_receive → command_chat /
//!   packet_action / chunk_send / entity_sync → inventory_sync → tick_begin →
//!   scheduler_tick → player_input → player_movement → entity_ai → physics →
//!   chunk_dirty_sync → tick_end → attribute_sync → network_send）
//! - [`app`]：自建 `App` 装配抽象（包裹 `World` + `Schedule`，替代 自建 App）
//! - [`prelude`]：ECS 预导出（与旧 ECS 同名项集中重导出，便于迁移）
//! - [`event`]：游戏循环与网络层之间的桥接事件（自研 `Message`）
//! - [`network`]：连接状态机与编解码 trait 骨架
//! - [`instance`]：区块 / 区段 / 世界容器（世界模型骨架）
//! - [`physics`]：AABB 碰撞盒与逐轴碰撞几何基础
//! - [`coordinate`]：坐标值类型（`Vec`/`BlockVec`/`Area`/`ChunkRange`）
//! - [`crypto`]：在线 Mojang 认证（WS5b，feature `online-auth`）——RSA-1024 握手 + AES-CFB8 + hasJoined 验证
//! - [`utils`]：通用工具（`MathUtils`/`TimeUtils`/`Validate`）
//! - [`thread`]：线程策略抽象（`ThreadProvider` 接口占位 + `StdThreadProvider`）
//!
//! ## 项目约束
//!
//! - Rust edition 2021，MSRV 1.89
//! - 全局 `#![forbid(unsafe_code)]`：禁止不安全代码
//! - 生产代码禁止 `unwrap()` / `expect()`、禁止裸 `[i]` 索引、禁止 `as` 缩窄转换
//! - 所有依赖版本精确锁定
//! - 基于自研 particlemc-framework-ecs 内核（替代旧 ECS 方案 app/ecs/time，`implement-custom-ecs` 变更标识符）

pub mod app;
pub mod component;
pub mod console;
pub mod coordinate;
pub mod crypto;
pub mod entity;
pub mod event;
pub mod extension;
pub mod instance;
pub mod inventory;
pub mod item_stack;
pub mod network;
pub mod physics;
pub mod plugin;
pub mod prelude;
pub mod protocol;
pub mod resource;
pub mod schedule;
pub mod system;
pub mod text_component;
pub mod thread;
pub mod utils;

#[cfg(test)]
pub mod test_support;
