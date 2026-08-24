// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 事件总线：按事件类型注册监听器并按注册顺序派发。
//!
//! [`EventBus`] 为 旧 ECS 方案 `Resource`，允许插件 / 应用注册任意类型事件的监听器
//! （[`Listener`]），并通过 [`EventBus::dispatch`] 派发事件。每个监听器收到
//! 一个 [`EventContext`]，可读取事件内容，或调用 [`EventContext::cancel`]
//! 取消后续监听器的调用。
//!
//! 派发时事件被克隆进上下文，监听器无法影响其他监听器可见的事件内容；任一
//! 监听器取消后，`dispatch` 立即停止，不再调用剩余监听器。

use std::any::{Any, TypeId};
use std::collections::HashMap;

/// 监听器唯一标识（`register` 的返回值，供 `unregister` 移除）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListenerId(u64);

/// 事件上下文：携带被派发的事件与取消标记。
pub struct EventContext<T> {
    /// 是否已被某个监听器取消。
    pub cancelled: bool,
    /// 被派发的事件（克隆）。
    pub event: T,
}

impl<T> EventContext<T> {
    /// 标记取消：`dispatch` 将跳过后续监听器。
    pub fn cancel(&mut self) {
        self.cancelled = true;
    }

    /// 是否已被取消。
    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }
}

/// 监听器 trait：处理 `T` 类型事件。
pub trait Listener<T>: Send + Sync {
    /// 处理一个事件（可读取 / 修改上下文，可取消）。
    fn handle(&mut self, ctx: &mut EventContext<T>);
}

/// 类型擦除监听器：`dispatch` 以 `&mut dyn Any` 传入上下文，
/// 具体实现向下转型到 `EventContext<T>` 后调用 [`Listener::handle`]。
///
/// 采用「注册时构造闭包」而非泛型 `impl`，避免未受约束类型参数的
/// E0207（Rust 不允许 where 子句中的类型参数作为唯一约束）。
#[allow(clippy::type_complexity)]
pub(crate) type ErasedListener = Box<dyn FnMut(&mut dyn Any) + Send + Sync>;

/// 事件总线（旧 ECS 方案 `Resource`）：按事件类型保存监听器并按注册顺序派发。
#[derive(Default)]
pub struct EventBus {
    /// 类型表：事件类型 → 有序监听器列表（保持注册顺序）。
    ///
    /// 监听器以「接收 `&mut dyn Any` 上下文」的闭包形式类型擦除存储；
    /// 具体 `Listener<T>` 在注册时被闭包捕获，派发时经向下转型还原。
    listeners: HashMap<TypeId, Vec<(ListenerId, ErasedListener)>>,
    /// 下一个可分配的监听器 id（全局递增）。
    next_id: u64,
}

impl EventBus {
    /// 为 `T` 注册一个监听器，返回其唯一标识。
    ///
    /// 同一类型的监听器按注册先后顺序被调用。
    pub fn register<T>(&mut self, listener: impl Listener<T> + 'static) -> ListenerId
    where
        T: Send + Sync + 'static,
    {
        let id = ListenerId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        let mut listener = listener;
        let erased: ErasedListener = Box::new(move |ctx: &mut dyn Any| {
            if let Some(typed) = ctx.downcast_mut::<EventContext<T>>() {
                listener.handle(typed);
            }
        });
        self.listeners
            .entry(TypeId::of::<T>())
            .or_default()
            .push((id, erased));
        id
    }

    /// 注销指定监听器；id 不存在时静默忽略。
    pub fn unregister<T>(&mut self, id: ListenerId)
    where
        T: Send + Sync + 'static,
    {
        if let Some(listeners) = self.listeners.get_mut(&TypeId::of::<T>()) {
            listeners.retain(|(listener_id, _)| *listener_id != id);
        }
    }

    /// 批量派发多个相同类型的事件：一次 HashMap 查找，遍历所有监听器×所有事件。
    ///
    /// 与 [`EventBus::dispatch`] 语义一致：每个事件独立处理取消标记。
    /// 某一事件的监听器链中若某个监听器取消，则跳过该事件剩余监听器，
    /// 但继续处理后续事件。
    pub fn dispatch_batch<T>(&mut self, events: &[T])
    where
        T: Send + Sync + Clone + 'static,
    {
        let Some(listeners) = self.listeners.get_mut(&TypeId::of::<T>()) else {
            return;
        };
        for event in events {
            let mut ctx = EventContext {
                cancelled: false,
                event: event.clone(),
            };
            for (_, listener) in listeners.iter_mut() {
                listener(&mut ctx);
                if ctx.cancelled {
                    break;
                }
            }
        }
    }

    /// 派发事件：按注册顺序调用监听器，任一取消则停止后续。
    ///
    /// 事件会被克隆进 [`EventContext`]；完成后上下文被丢弃。
    pub fn dispatch<T>(&mut self, event: T)
    where
        T: Send + Sync + Clone + 'static,
    {
        let Some(listeners) = self.listeners.get_mut(&TypeId::of::<T>()) else {
            return;
        };
        let mut ctx = EventContext {
            cancelled: false,
            event,
        };
        for (_, listener) in listeners.iter_mut() {
            listener(&mut ctx);
            if ctx.cancelled {
                break;
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// 记录被调用顺序的 `i32` 监听器。
    #[derive(Clone)]
    struct Recording {
        log: Arc<Mutex<Vec<u32>>>,
        id: u32,
    }

    impl Listener<i32> for Recording {
        fn handle(&mut self, _ctx: &mut EventContext<i32>) {
            self.log.lock().unwrap().push(self.id);
        }
    }

    /// 立即取消后续监听的 `i32` 监听器。
    struct Canceller;

    impl Listener<i32> for Canceller {
        fn handle(&mut self, ctx: &mut EventContext<i32>) {
            ctx.cancel();
        }
    }

    /// 记录被调用顺序的 `String` 监听器（验证类型隔离）。
    #[derive(Clone)]
    struct StringRecording {
        log: Arc<Mutex<Vec<u32>>>,
        id: u32,
    }

    impl Listener<String> for StringRecording {
        fn handle(&mut self, _ctx: &mut EventContext<String>) {
            self.log.lock().unwrap().push(self.id);
        }
    }

    #[test]
    fn listeners_called_in_registration_order() {
        let mut bus = EventBus::default();
        let log = Arc::new(Mutex::new(Vec::new()));
        bus.register(Recording {
            log: log.clone(),
            id: 1,
        });
        bus.register(Recording {
            log: log.clone(),
            id: 2,
        });
        bus.register(Recording {
            log: log.clone(),
            id: 3,
        });
        bus.dispatch(42);
        assert_eq!(*log.lock().unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn cancel_stops_later_listeners() {
        let mut bus = EventBus::default();
        let log = Arc::new(Mutex::new(Vec::new()));
        bus.register(Recording {
            log: log.clone(),
            id: 1,
        });
        bus.register(Canceller);
        bus.register(Recording {
            log: log.clone(),
            id: 3,
        });
        bus.dispatch(42);
        assert_eq!(*log.lock().unwrap(), vec![1]);
    }

    #[test]
    fn unregister_prevents_future_delivery() {
        let mut bus = EventBus::default();
        let log = Arc::new(Mutex::new(Vec::new()));
        let first = bus.register(Recording {
            log: log.clone(),
            id: 1,
        });
        bus.register(Recording {
            log: log.clone(),
            id: 2,
        });
        bus.unregister::<i32>(first);
        bus.dispatch(42);
        assert_eq!(*log.lock().unwrap(), vec![2]);
        // 对不存在的 id 注销应静默忽略。
        bus.unregister::<i32>(ListenerId(999));
    }

    #[test]
    fn different_types_are_isolated() {
        let mut bus = EventBus::default();
        let log = Arc::new(Mutex::new(Vec::new()));
        bus.register(Recording {
            log: log.clone(),
            id: 1,
        });
        bus.register(StringRecording {
            log: log.clone(),
            id: 2,
        });
        // 仅派发 i32：String 监听器不应被触发。
        bus.dispatch(42);
        assert_eq!(*log.lock().unwrap(), vec![1]);
    }

    #[test]
    fn dispatch_without_listeners_is_noop() {
        let mut bus = EventBus::default();
        bus.dispatch(42);
        bus.dispatch("hello".to_string());
    }

    #[test]
    fn listener_can_read_cloned_event() {
        struct Capture {
            got: Arc<Mutex<Option<String>>>,
        }
        impl Listener<String> for Capture {
            fn handle(&mut self, ctx: &mut EventContext<String>) {
                *self.got.lock().unwrap() = Some(ctx.event.clone());
            }
        }
        let mut bus = EventBus::default();
        let got = Arc::new(Mutex::new(None));
        bus.register(Capture { got: got.clone() });
        bus.dispatch("world".to_string());
        assert_eq!(*got.lock().unwrap(), Some("world".to_string()));
    }

    #[test]
    fn dispatch_batch_calls_all_listeners_for_all_events() {
        let mut bus = EventBus::default();
        let log = Arc::new(Mutex::new(Vec::new()));
        bus.register(Recording {
            log: log.clone(),
            id: 1,
        });
        bus.register(Recording {
            log: log.clone(),
            id: 2,
        });
        // 两个事件，每个都触发两个监听器。
        bus.dispatch_batch(&[10, 20]);
        assert_eq!(*log.lock().unwrap(), vec![1, 2, 1, 2]);
    }

    #[test]
    fn dispatch_batch_cancel_stops_later_listeners_of_same_event() {
        let mut bus = EventBus::default();
        let log = Arc::new(Mutex::new(Vec::new()));
        bus.register(Recording {
            log: log.clone(),
            id: 1,
        });
        bus.register(Canceller);
        bus.register(Recording {
            log: log.clone(),
            id: 3,
        });
        // 第一事件：listener 1 触发后 Canceller 取消，listener 3 被跳过；
        // 第二事件独立处理：listener 1 触发后 Canceller 取消，listener 3 仍被跳过。
        bus.dispatch_batch(&[42, 99]);
        assert_eq!(*log.lock().unwrap(), vec![1, 1]);
    }

    #[test]
    fn dispatch_batch_empty_slice_is_noop() {
        let mut bus = EventBus::default();
        bus.dispatch_batch::<i32>(&[]);
    }

    #[test]
    fn dispatch_batch_without_listeners_is_noop() {
        let mut bus = EventBus::default();
        bus.dispatch_batch(&[1, 2, 3]);
        bus.dispatch_batch(&["a".to_string(), "b".to_string()]);
    }
}
