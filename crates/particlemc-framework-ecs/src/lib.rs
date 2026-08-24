// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 自研 ECS 内核 crate：面向 Minecraft 服务端的极致性能实体组件系统。
//!
//! 变更标识符：`implement-custom-ecs`
//!
//! 本 crate 替代既有 自建 App / 自研 ECS / 自研时钟 依赖（RM1），对齐
//! Minestom 框架层语义（静态 Archetype、世界级隔离 + 全局实例调度器、
//! 跨线程命令队列），同时保持高层 API 形态（`Component` / `Resource` /
//! `World` 等）与 旧 ECS 方案 兼容，便于 particlemc-framework-core 迁移。
//!
//! 本任务（T1）提供以下模块：
//! - [`entity`]：实体句柄（u64 分段编码：类型 ID / 世代 / 槽位）与按类型隔离的槽位分配器 [`entity::EntityArena`]
//! - [`component`]：组件注册契约（[`component::Component`] trait）与惰性全局 [`component::ComponentId`] 分配
//! - [`resource`]：跨系统共享单例数据（[`resource::Resource`] trait）与按 `TypeId` 索引的资源表
//! - [`world`]：独立 ECS 世界（本任务仅含资源存储；实体 CRUD 由 T3 补充）
//! - [`util`]：通用工具（缓存行对齐包装、2 的幂扩容辅助）
//!
//! 后续任务将追加 storage / archetype / query / commands / message / schedule
//! 等模块，本 crate 全局禁止 `unsafe`（白名单模块在后续任务局部放行）。

#![deny(unsafe_code)]
// 宪章允许测试代码使用 `unwrap`/`expect`（见 constitution.md「unwrap/expect 禁令」一节）。
// 仅在测试构建（`cargo clippy --all-targets` 的 test target）放宽为允许；生产构建仍受
// `-D clippy::unwrap_used -D clippy::expect_used` 硬门禁约束，故生产代码不得出现 unwrap/expect。
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod archetype;
pub mod commands; // T5 提供（延迟命令缓冲，IC-6）
pub mod component;
pub mod entity;
pub mod message;
pub mod migration; // T8 提供（实体跨世界迁移原语，IC-12）
pub mod query; // T4a 提供（只读 Query 迭代 + 确定性匹配）
pub mod queue; // T8 提供（lock-free MPMC 命令队列，IC-11）
pub mod resource;
pub mod schedule; // T7b 提供（Schedule / FixedClock / 依赖排序 / 消息清空，IC-9）
pub mod scheduler; // T9 提供（InstanceScheduler 全局调度器，IC-10）
pub mod shared; // T8 提供（共享只读资源，IC-13）
pub mod storage;
pub mod system; // T7a 提供（System / SystemParam / FunctionSystem，IC-7/IC-9）
pub mod util;
pub mod world;
