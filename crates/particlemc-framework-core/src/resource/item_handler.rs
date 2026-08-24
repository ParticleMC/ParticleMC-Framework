//! 物品处理器（R11）：`ItemHandler` 回调 trait 与注册表。
//!
//! 应用侧可为特定 `material` 注册一个 [`ItemHandler`]，在使用 / 持有 / 丢弃
//! 物品时收到回调。回调的具体派发路径（tick 管线中的触发点）由后续任务接入；
//! 本模块只提供注册、查询与缺省行为。处理器以 [`Arc<dyn ItemHandler>`] 存储，
//! 可跨线程共享（trait 约束 `Send + Sync`）。
//!
//! `ItemStack` 刻意保持值类型语义（不增加 handler 字段，避免破坏既有
//! `Clone`/`PartialEq` 与线格式编解码），handler 以
//! [`ItemHandlerRegistry`]（`HashMap<material, Arc<dyn ItemHandler>>`）独立承载。
//!
//! 变更标识符：`complete-missing-subsystems`（R11 item 子包行为 API）。

use std::collections::HashMap;
use std::sync::Arc;

use crate::prelude::{Entity, World};

use crate::item_stack::ItemStack;

/// 物品处理器 trait：物品生命周期事件的回调接口。
///
/// `on_use` 为必实现入口；`on_hold` / `on_drop` 提供默认空实现，实现方只需
/// 覆盖关心的事件。回调不持有 系统参数，由派发方构造 `&mut World` 传入。
pub trait ItemHandler: Send + Sync {
    /// 物品被使用（右键 / 食用等）时回调。
    fn on_use(&self, player: Entity, world: &mut World, item: &ItemStack);
    /// 物品被持有（切换到主手 / 副手）时回调。
    fn on_hold(&self, _player: Entity, _world: &mut World, _item: &ItemStack) {}
    /// 物品被丢弃时回调。
    fn on_drop(&self, _player: Entity, _world: &mut World, _item: &ItemStack) {}
}

/// 物品处理器注册表（旧 ECS 方案 `Resource`）。
///
/// 以协议物品 id（`ItemStack::material`）为键存储 handler；同一 material 重复
/// 注册会替换旧 handler 并返回被替换者。`get` 仅按 material 查询，不做
/// `ItemStack` 值语义比较。
#[derive(Default)]
pub struct ItemHandlerRegistry {
    /// material id → 处理器。
    handlers: HashMap<u32, Arc<dyn ItemHandler>>,
}

impl ItemHandlerRegistry {
    /// 构造一个空注册表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册（或替换）某 material 的处理器，返回被替换的旧处理器（无则 `None`）。
    pub fn register(
        &mut self,
        material: u32,
        handler: Arc<dyn ItemHandler>,
    ) -> Option<Arc<dyn ItemHandler>> {
        self.handlers.insert(material, handler)
    }

    /// 按 material 查询处理器。
    pub fn get(&self, material: u32) -> Option<&Arc<dyn ItemHandler>> {
        self.handlers.get(&material)
    }

    /// 按物品栈的 material 查询处理器（便捷方法）。
    pub fn handler_for(&self, item: &ItemStack) -> Option<&Arc<dyn ItemHandler>> {
        self.get(item.material)
    }

    /// 已注册处理器数量。
    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    /// 注册表是否为空。
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// 记录型测试处理器：统计各回调触发次数并记录收到事件的 material。
    #[derive(Default)]
    struct RecordingHandler {
        uses: AtomicUsize,
        holds: AtomicUsize,
        drops: AtomicUsize,
        seen: Mutex<Vec<u32>>,
    }

    impl RecordingHandler {
        fn counts(&self) -> (usize, usize, usize) {
            (
                self.uses.load(Ordering::SeqCst),
                self.holds.load(Ordering::SeqCst),
                self.drops.load(Ordering::SeqCst),
            )
        }
    }

    impl ItemHandler for RecordingHandler {
        fn on_use(&self, _player: Entity, _world: &mut World, item: &ItemStack) {
            self.uses.fetch_add(1, Ordering::SeqCst);
            self.seen.lock().unwrap().push(item.material);
        }
        fn on_hold(&self, _player: Entity, _world: &mut World, item: &ItemStack) {
            self.holds.fetch_add(1, Ordering::SeqCst);
            self.seen.lock().unwrap().push(item.material);
        }
    }

    #[test]
    fn register_get_and_handler_for() {
        let mut registry = ItemHandlerRegistry::new();
        assert!(registry.is_empty());
        let handler: Arc<dyn ItemHandler> = Arc::new(RecordingHandler::default());
        assert!(registry.register(264, handler).is_none());
        assert_eq!(registry.len(), 1);
        assert!(registry.get(264).is_some());
        assert!(registry.get(1).is_none());
        // handler_for 按物品 material 查询。
        let diamond = ItemStack::new(264, 1);
        assert!(registry.handler_for(&diamond).is_some());
        let stone = ItemStack::new(1, 1);
        assert!(registry.handler_for(&stone).is_none());
    }

    #[test]
    fn register_replaces_old_handler_and_returns_it() {
        let mut registry = ItemHandlerRegistry::new();
        let old: Arc<dyn ItemHandler> = Arc::new(RecordingHandler::default());
        let new: Arc<dyn ItemHandler> = Arc::new(RecordingHandler::default());
        registry.register(264, old);
        let replaced = registry.register(264, new);
        assert!(replaced.is_some());
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn on_use_invoked_with_world_and_item() {
        // 验证回调可被派发（构造 `World` 与实体），且 on_hold 覆盖生效。
        let mut world = World::new();
        let player = world.spawn_empty().id();
        let recording = Arc::new(RecordingHandler::default());
        let handler: Arc<dyn ItemHandler> = recording.clone();
        let item = ItemStack::new(264, 1);
        handler.on_use(player, &mut world, &item);
        handler.on_hold(player, &mut world, &item);
        assert_eq!(recording.counts(), (1, 1, 0));
        assert_eq!(*recording.seen.lock().unwrap(), vec![264, 264]);
    }

    #[test]
    fn default_callbacks_are_noop() {
        // on_hold / on_drop 默认空实现：不 panic、不产生副作用。
        let mut world = World::new();
        let player = world.spawn_empty().id();
        let recording = Arc::new(RecordingHandler::default());
        let handler: Arc<dyn ItemHandler> = recording.clone();
        let item = ItemStack::new(264, 1);
        handler.on_drop(player, &mut world, &item);
        assert_eq!(recording.counts(), (0, 0, 0));
    }

    #[test]
    fn unknown_material_returns_none() {
        let mut registry = ItemHandlerRegistry::new();
        registry.register(264, Arc::new(RecordingHandler::default()));
        assert!(registry.get(0).is_none());
        assert!(registry.get(u32::MAX).is_none());
    }
}
