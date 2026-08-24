// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 事件 Trait 层次体系。
//!
//! 对应 Java Minestom 的 `Event` / `CancellableEvent` / `AsyncEvent` /
//! `InstanceEvent` / `EntityEvent` / `PlayerEvent` / `BlockEvent` 继承链。
//!
//! 所有事件均派生 `Message`（自研 ECS）+ `Debug` + `Clone`，本模块提供额外的
//! trait 组合以支持过滤、取消、异步派发等语义。

use crate::prelude::{Entity, Message};

use crate::component::Position;
use particlemc_framework_ecs::scheduler::WorldId;

/// 方块类型（来自注册表）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Block {
    /// 方块 id（来自注册表）。
    pub id: u32,
}

/// 事件基 trait（对应 Java `Event`）。
///
/// 所有事件必须实现此 trait；它强制要求事件具备 `Send + Sync + Clone + 'static`
/// 约束，以满足多线程分发与跨 tick 存储的需求。
pub trait Event: Send + Sync + Clone + Message + 'static {}

/// 可取消事件（对应 Java `CancellableEvent`）。
///
/// 提供 `is_cancelled` / `set_cancelled` 接口；监听器可在取消后跳过后续处理。
/// 通常事件结构体内含 `cancelled: bool` 字段，由 dispatch 框架自动更新。
pub trait CancellableEvent: Event {
    /// 返回事件是否已被取消。
    fn is_cancelled(&self) -> bool;
    /// 设置事件的取消状态。
    fn set_cancelled(&mut self, cancelled: bool);
}

/// 异步事件（对应 Java `AsyncEvent`）。
///
/// feature `async-events` 开启时，dispatch 框架会在专用线程池中执行监听器，
/// 避免阻塞主游戏循环。默认关闭，监听器在调用线程同步执行。
#[cfg(feature = "async-events")]
pub trait AsyncEvent: Event {}

/// 实例相关事件（对应 Java `InstanceEvent`）。
///
/// 持有实例引用，用于按实例过滤监听器（实例级事件节点）。
pub trait InstanceEvent: Event {
    /// 返回事件所属实例的世界 id；未关联实例时返回 `None`。
    fn instance_id(&self) -> Option<WorldId>;
}

/// 实体相关事件（对应 Java `EntityEvent`）。
///
/// 持有实体引用，用于按实体过滤监听器。
pub trait EntityEvent: Event {
    /// 返回事件关联的实体。
    fn entity(&self) -> Entity;
}

/// 玩家相关事件（对应 Java `PlayerEvent`）。
///
/// 继承 [`EntityEvent`]，限定事件主体为玩家实体。
pub trait PlayerEvent: EntityEvent {
    /// 返回玩家实体 id（与 `entity()` 相同）。
    fn player(&self) -> Entity {
        self.entity()
    }
}

/// 方块相关事件（对应 Java `BlockEvent`）。
///
/// 继承 [`InstanceEvent`]，提供方块位置与方块类型访问。
pub trait BlockEvent: InstanceEvent {
    /// 返回事件涉及的方块位置。
    fn block_position(&self) -> Position;
    /// 返回事件涉及的方块（由 `BlockRegistry` 查询）。
    fn block(&self) -> Block {
        Block::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::Entity;

    /// 测试用事件结构体（验证 trait 可实现性）。
    #[derive(Debug, Clone, Message)]
    struct TestEntityEvent {
        entity: Entity,
    }

    impl Event for TestEntityEvent {}

    impl EntityEvent for TestEntityEvent {
        fn entity(&self) -> Entity {
            self.entity
        }
    }

    #[test]
    fn test_event_trait_impl() {
        let evt = TestEntityEvent {
            entity: Entity::from_raw_u32(42),
        };
        assert_eq!(evt.entity().index_u32(), 42);
    }
}
