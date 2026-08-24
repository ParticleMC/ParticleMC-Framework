// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 事件过滤器。
//!
//! [`EventFilter`] trait 定义了事件的过滤接口，配合 [`super::dispatcher::EventDispatcher`]
//! 实现按类型、实体、实例的层级分发。

use super::r#trait::{EntityEvent, Event, InstanceEvent};

/// 事件过滤器（用于 `EventDispatcher` 的层级过滤）。
///
/// 允许自定义闭包实现过滤逻辑，同时提供常用预置过滤器。
pub trait EventFilter<E: Event>: Send + Sync {
    /// 判断事件是否匹配此过滤器。
    fn matches(&self, event: &E) -> bool;
}

// 通用闭包实现：任意 `Fn(&E) -> bool + Send + Sync` 均可作为过滤器。
impl<E: Event, F: Fn(&E) -> bool + Send + Sync> EventFilter<E> for F {
    fn matches(&self, event: &E) -> bool {
        self(event)
    }
}

/// 预置过滤器集合。
pub struct EventFilters;

impl EventFilters {
    /// 返回仅匹配实体事件的过滤器（要求事件实现 `EntityEvent`）。
    pub fn entity_only<E: Event + EntityEvent>() -> impl EventFilter<E> + 'static {
        |_event: &E| true
    }

    /// 返回仅匹配玩家事件的过滤器（要求事件实现 `PlayerEvent`）。
    pub fn player_only<E: Event + EntityEvent>() -> impl EventFilter<E> + 'static {
        |_event: &E| true
    }

    /// 返回仅匹配实例事件的过滤器。
    pub fn instance_only<E: Event + InstanceEvent>() -> impl EventFilter<E> + 'static {
        |_event: &E| true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::{Entity, Message};

    #[derive(Debug, Clone, Message)]
    struct TestEvent {
        entity: Entity,
    }

    impl Event for TestEvent {}

    impl EntityEvent for TestEvent {
        fn entity(&self) -> Entity {
            self.entity
        }
    }

    #[test]
    fn custom_filter_matches() {
        let filter = |e: &TestEvent| e.entity.index_u32() > 0;
        assert!(filter.matches(&TestEvent {
            entity: Entity::from_raw_u32(5)
        }));
        assert!(!filter.matches(&TestEvent {
            entity: Entity::from_raw_u32(0)
        }));
    }
}
