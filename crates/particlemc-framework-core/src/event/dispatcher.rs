// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 事件分发器（EventDispatcher）。
//!
//! 支持全局监听 + 实例级节点分层，替代旧版 [`super::bus::EventBus`]。
//! 保留 `EventBus` 作为 deprecated 兼容层，提供 `into_dispatcher()` 转换。

use std::any::TypeId;
use std::collections::HashMap;

use super::r#trait::{Event, InstanceEvent};
use particlemc_framework_ecs::scheduler::WorldId;

type AnyFn = Box<dyn Fn(&dyn std::any::Any) + Send + Sync>;

/// 监听器 id（用于注销）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListenerId(u64);

impl ListenerId {
    pub(crate) fn new(id: u64) -> Self {
        Self(id)
    }
}

/// 类型擦除的监听器包装。
struct ErasedListener {
    call_fn: AnyFn,
}

/// 事件分发器（支持全局 + 实例级节点）。
pub struct EventDispatcher {
    global_listeners: HashMap<TypeId, Vec<(ListenerId, ErasedListener)>>,
    nodes: HashMap<WorldId, EventDispatcher>,
    next_id: u64,
}

impl Default for EventDispatcher {
    fn default() -> Self {
        Self {
            global_listeners: HashMap::new(),
            nodes: HashMap::new(),
            next_id: 1,
        }
    }
}

impl EventDispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册全局监听器（返回 id 用于注销）。
    pub fn register<E: Event>(
        &mut self,
        listener: impl Fn(E) + Send + Sync + 'static,
    ) -> ListenerId {
        let id = ListenerId::new(self.next_id);
        self.next_id += 1;
        let erased = ErasedListener {
            call_fn: Box::new(move |any: &dyn std::any::Any| {
                if let Some(event) = any.downcast_ref::<E>() {
                    listener(event.clone());
                }
            }),
        };
        self.global_listeners
            .entry(TypeId::of::<E>())
            .or_default()
            .push((id, erased));
        id
    }

    /// 注销监听器。
    pub fn unregister<E: Event>(&mut self, id: ListenerId) {
        if let Some(list) = self.global_listeners.get_mut(&TypeId::of::<E>()) {
            list.retain(|(lid, _)| *lid != id);
        }
    }

    /// 派发全局事件（不区分实例）。
    pub fn dispatch<E: Event + InstanceEvent>(&mut self, event: E) {
        if let Some(list) = self.global_listeners.get(&TypeId::of::<E>()) {
            for (_, listener) in list {
                (listener.call_fn)(&event as &dyn std::any::Any);
            }
        }

        // 若有实例信息，同步分发给实例节点
        #[allow(clippy::collapsible_if)]
        if let Some(node_id) = event.instance_id() {
            if let Some(node) = self.nodes.get_mut(&node_id) {
                node.dispatch(event);
            }
        }
    }

    pub fn add_node(&mut self, world_id: WorldId, node: EventDispatcher) {
        self.nodes.insert(world_id, node);
    }

    pub fn remove_node(&mut self, world_id: WorldId) {
        self.nodes.remove(&world_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::{Entity, Message};
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone, Message)]
    struct TestEvent {
        entity: Entity,
    }

    impl Event for TestEvent {}

    impl InstanceEvent for TestEvent {
        fn instance_id(&self) -> Option<WorldId> {
            None
        }
    }

    #[derive(Debug, Clone, Message)]
    struct InstanceTestEvent {
        world_id: Option<WorldId>,
    }

    impl Event for InstanceTestEvent {}

    impl InstanceEvent for InstanceTestEvent {
        fn instance_id(&self) -> Option<WorldId> {
            self.world_id
        }
    }

    #[test]
    fn dispatch_to_global_listeners() {
        let mut dispatcher = EventDispatcher::new();
        let captured = Arc::new(Mutex::new(Vec::new()));

        dispatcher.register::<TestEvent>({
            let captured = captured.clone();
            move |e: TestEvent| {
                captured.lock().unwrap().push(e.entity.index_u32());
            }
        });

        dispatcher.dispatch(TestEvent {
            entity: Entity::from_raw_u32(1),
        });
        dispatcher.dispatch(TestEvent {
            entity: Entity::from_raw_u32(2),
        });

        assert_eq!(*captured.lock().unwrap(), vec![1, 2]);
    }

    #[test]
    fn dispatch_to_instance_node() {
        let mut dispatcher = EventDispatcher::new();
        let captured = Arc::new(Mutex::new(Vec::new()));

        let inst_id = WorldId(42);
        let mut node = EventDispatcher::new();
        node.register::<InstanceTestEvent>({
            let captured = captured.clone();
            move |e: InstanceTestEvent| {
                captured.lock().unwrap().push(e.world_id);
            }
        });
        dispatcher.add_node(inst_id, node);

        dispatcher.dispatch(InstanceTestEvent {
            world_id: Some(inst_id),
        });

        assert_eq!(*captured.lock().unwrap(), vec![Some(inst_id)]);
    }
}
