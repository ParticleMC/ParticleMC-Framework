//! particlemc-framework-core 的 ECS 预导出模块。
//!
//! 变更标识符：`implement-custom-ecs`（T10）
//!
//! 本模块把 `particlemc_framework_ecs` 中与旧 ECS 同名的核心 ECS 项集中重导出，使既有代码
//! 的 `use crate::prelude::*` 可机械替换为 `use crate::prelude::*`，最小化
//! 迁移面（T11）。`Component` / `Message` 的 **trait 与派生宏同名共存**（不同
//! 命名空间），与 旧 ECS 方案 的 prelude 形态一致。

// ---- 组件 / 资源 / 消息 trait + 派生宏 ----
pub use particlemc_framework_ecs::component::{Component, ComponentId, ComponentStorage};
pub use particlemc_framework_ecs::message::Message;
pub use particlemc_framework_ecs::resource::Resource;

// 派生宏（proc-macro crate）：与上方 trait 同名，位于宏命名空间，可共存。
pub use particlemc_framework_ecs_macros::{Component, Message};

// ---- 系统参数 ----
pub use particlemc_framework_ecs::commands::Commands;
pub use particlemc_framework_ecs::message::{MessageInbox, MessageReader, MessageWriter};
pub use particlemc_framework_ecs::query::{Query, With, Without};
pub use particlemc_framework_ecs::system::{Res, ResMut};

// ---- 实体 / 世界 / 装配 ----
pub use particlemc_framework_ecs::archetype::ArchetypeId;
pub use particlemc_framework_ecs::entity::{Entity, EntityTypeId};
pub use particlemc_framework_ecs::schedule::{FixedClock, Schedule};
pub use particlemc_framework_ecs::shared::Shared;
pub use particlemc_framework_ecs::world::World;

// 阶段标记与 App 装配（自建，替代 自建 App）。
pub use crate::app::{App, FixedUpdate, Plugin, TimeUpdateStrategy, Update};
