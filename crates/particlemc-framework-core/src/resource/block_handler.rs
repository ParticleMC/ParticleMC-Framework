//! 方块处理器：方块生命周期事件的注册与查询。
//!
//! 应用侧可为特定方块注册一个 [`BlockHandler`]，在放置 / 破坏 / 交互事件
//! 触发时收到回调。回调事件的具体派发路径（tick 管线中的触发点）由后续
//! 任务接入；本模块只提供注册、查询与缺省行为。处理器以
//! [`Arc<dyn BlockHandler>`] 存储，可跨线程共享（trait 约束 `Send + Sync`）。
//!
//! 存储结构采用以方块状态 id 为下标的 `Vec<Option<Arc<dyn BlockHandler>>>`：
//! [`register_by_id`](Self::register_by_id) / [`handler_by_id`](Self::handler_by_id)
//! 直接按 id 操作，性能稳定；[`register`](Self::register) /
//! [`handler`](Self::handler) 为名称别名，内部依赖可选的
//! [`BlockRegistry`](crate::resource::registries::BlockRegistry) 完成
//! name → state_id 解析。未设置注册表时，名称别名返回 `None`。

use std::sync::Arc;

use crate::prelude::Entity;

use crate::component::{Block, Position};
use crate::resource::registries::BlockRegistry;

/// 方块放置上下文：放置位置与被放置的方块。
pub struct BlockPlaceContext {
    /// 所属实例实体。
    pub instance: Entity,
    /// 放置位置。
    pub position: Position,
    /// 被放置的方块。
    pub block: Block,
}

/// 方块破坏上下文：破坏位置与被破坏的方块。
pub struct BlockBreakContext {
    /// 所属实例实体。
    pub instance: Entity,
    /// 破坏位置。
    pub position: Position,
    /// 被破坏的方块。
    pub block: Block,
}

/// 方块交互上下文：交互位置、方块与交互玩家。
pub struct BlockInteractContext {
    /// 所属实例实体。
    pub instance: Entity,
    /// 交互位置。
    pub position: Position,
    /// 被交互的方块。
    pub block: Block,
    /// 发起交互的玩家实体。
    pub player: Entity,
}

/// 方块处理器 trait：方块生命周期事件的回调接口。
///
/// 全部回调提供默认空实现，实现方只需覆盖关心的事件。
pub trait BlockHandler: Send + Sync {
    /// 方块放置时回调。
    fn on_place(&self, _ctx: &mut BlockPlaceContext) {}
    /// 方块破坏时回调。
    fn on_break(&self, _ctx: &mut BlockBreakContext) {}
    /// 方块交互时回调。
    fn on_interact(&self, _ctx: &mut BlockInteractContext) {}
}

/// 方块处理器注册表（旧 ECS 方案 `Resource`）。
///
/// 以方块状态 id 为下标存储处理器，`Vec<Option<...>>` 支持稀疏 id 分配。
/// `Default` 构造空表，由 [`crate::plugin::McServerPlugin`] 装配或在应用侧
/// 自行插入。可选注册表字段仅用于名称别名方法的 id 解析。
#[derive(Default)]
pub struct BlockHandlers {
    /// 以方块状态 id 为下标的处理器槽位表。
    ///
    /// 下标 = state_id，元素为 `Some(handler)` 或 `None`（未注册）。
    map: Vec<Option<Arc<dyn BlockHandler>>>,
    /// 方块注册表（可选）。
    ///
    /// 仅在调用 [`register`](Self::register) / [`handler`](Self::handler) /
    /// [`remove`](Self::remove) 等名称别名时需要；直接按 id 操作的 API
    /// 不依赖此字段。
    registry: Option<BlockRegistry>,
}

impl BlockHandlers {
    /// 构造一个空处理器注册表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 绑定方块注册表，使名称别名方法（[`register`](Self::register)、
    /// [`handler`](Self::handler)、[`remove`](Self::remove)）可用。
    ///
    /// 返回自身，便于链式调用。
    pub fn with_registry(mut self, registry: BlockRegistry) -> Self {
        self.registry = Some(registry);
        self
    }

    /// 注册（或替换）指定方块状态 id 的处理器。
    ///
    /// 若 `state_id` 超出当前表长度，自动填充 `None` 槽位后写入。
    pub fn register_by_id(&mut self, state_id: u32, handler: Arc<dyn BlockHandler>) {
        let new_len = (state_id + 1) as usize;
        if self.map.len() < new_len {
            self.map.resize_with(new_len, || None);
        }
        self.map[state_id as usize] = Some(handler);
    }

    /// 查询指定方块状态 id 的处理器；未注册时返回 `None`。
    pub fn handler_by_id(&self, state_id: u32) -> Option<&Arc<dyn BlockHandler>> {
        self.map.get(state_id as usize).and_then(|slot| slot.as_ref())
    }

    /// 移除指定方块状态 id 的处理器，返回被移除的处理器（若有）。
    pub fn remove_by_id(&mut self, state_id: u32) -> Option<Arc<dyn BlockHandler>> {
        self.map.get_mut(state_id as usize).and_then(|slot| slot.take())
    }

    /// 注册（或替换）指定命名空间名称的方块处理器。
    ///
    /// 需要已绑定 [`BlockRegistry`](crate::resource::registries::BlockRegistry)，
    /// 否则返回 `None`（名称无法解析）。
    pub fn register(&mut self, name: &str, handler: Arc<dyn BlockHandler>) -> bool {
        let registry = match &self.registry {
            Some(r) => r,
            None => return false,
        };
        match registry.block_from_name(name) {
            Some(block) => {
                self.register_by_id(block.state_id(), handler);
                true
            }
            None => false,
        }
    }

    /// 查询指定命名空间名称的处理器；未注册或名称无法解析时返回 `None`。
    pub fn handler(&self, name: &str) -> Option<&Arc<dyn BlockHandler>> {
        let block = self.registry.as_ref()?.block_from_name(name)?;
        self.handler_by_id(block.state_id())
    }

    /// 移除指定命名空间名称的处理器，返回被移除的处理器（若有）。
    ///
    /// 名称无法解析时返回 `None`。
    pub fn remove(&mut self, name: &str) -> Option<Arc<dyn BlockHandler>> {
        let block = self.registry.as_ref()?.block_from_name(name)?;
        self.remove_by_id(block.state_id())
    }

    /// 已注册处理器数量（`Some` 槽位数）。
    pub fn len(&self) -> usize {
        self.map.iter().filter(|slot| slot.is_some()).count()
    }

    /// 是否没有任何处理器。
    pub fn is_empty(&self) -> bool {
        self.map.iter().all(|slot| slot.is_none())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;
    use crate::resource::registries::{BlockDefinition, Registry};

    /// 测试处理器：通过共享 `Arc<AtomicBool>` 记录回调是否被调用。
    struct FlagHandler {
        broken: Arc<AtomicBool>,
    }

    impl BlockHandler for FlagHandler {
        fn on_break(&self, _ctx: &mut BlockBreakContext) {
            self.broken.store(true, Ordering::SeqCst);
        }
    }

    /// 构造最小测试注册表：air=0、stone=1、dirt=2、chest=3。
    fn test_registry() -> BlockRegistry {
        let toml = r#"
            [[entry]]
            id = 0
            name = "minecraft:air"

            [[entry]]
            id = 1
            name = "minecraft:stone"

            [[entry]]
            id = 2
            name = "minecraft:dirt"

            [[entry]]
            id = 3
            name = "minecraft:chest"
        "#;
        BlockRegistry(Registry::<BlockDefinition>::from_toml_str(toml).unwrap())
    }

    #[test]
    fn register_by_id_query_and_default_none() {
        let mut handlers = BlockHandlers::new();
        assert!(handlers.is_empty());
        assert!(handlers.handler_by_id(0).is_none());

        let handler: Arc<dyn BlockHandler> = Arc::new(FlagHandler {
            broken: Arc::new(AtomicBool::new(false)),
        });
        handlers.register_by_id(5, Arc::clone(&handler));
        assert_eq!(handlers.len(), 1);
        // 稀疏 id：中间槽位均为 None。
        assert!(handlers.handler_by_id(3).is_none());
        assert!(handlers.handler_by_id(5).is_some());

        // 覆盖注册同名 id，不增加条目数。
        handlers.register_by_id(5, Arc::clone(&handler));
        assert_eq!(handlers.len(), 1);

        // 移除后查询返回 None。
        assert!(handlers.remove_by_id(5).is_some());
        assert!(handlers.handler_by_id(5).is_none());
        assert!(handlers.is_empty());
    }

    #[test]
    fn register_by_name_requires_registry() {
        let mut handlers = BlockHandlers::new();
        // 未绑定注册表：名称别名方法恒返回 false/None。
        assert!(!handlers.register("minecraft:stone", Arc::new(FlagHandler {
            broken: Arc::new(AtomicBool::new(false)),
        })));
        assert!(handlers.handler("minecraft:stone").is_none());
        assert!(handlers.remove("minecraft:stone").is_none());
        assert!(handlers.is_empty());
    }

    #[test]
    fn register_by_name_resolves_via_registry() {
        let registry = test_registry();
        let mut handlers = BlockHandlers::new().with_registry(registry.clone());

        let handler: Arc<dyn BlockHandler> = Arc::new(FlagHandler {
            broken: Arc::new(AtomicBool::new(false)),
        });
        assert!(handlers.register("minecraft:chest", Arc::clone(&handler)));
        assert_eq!(handlers.len(), 1);
        // chest 的 state_id = 3，handler_by_id 可按 id 查到。
        assert!(handlers.handler_by_id(3).is_some());
        // 名称别名亦可查到。
        assert!(handlers.handler("minecraft:chest").is_some());
        assert!(handlers.handler("minecraft:missing").is_none());

        // 覆盖注册同名，不增加条目数。
        handlers.register("minecraft:chest", Arc::clone(&handler));
        assert_eq!(handlers.len(), 1);

        // 移除后查询返回 None。
        assert!(handlers.remove("minecraft:chest").is_some());
        assert!(handlers.handler("minecraft:chest").is_none());
        assert!(handlers.is_empty());
    }

    #[test]
    fn default_callbacks_are_noop_and_do_not_panic() {
        let handlers = BlockHandlers::new();
        // 未注册处理器：回调不触发、也不 panic。
        let handler = Arc::new(FlagHandler {
            broken: Arc::new(AtomicBool::new(false)),
        });
        handler.on_place(&mut BlockPlaceContext {
            instance: Entity::PLACEHOLDER,
            position: Position::new(1.0, 2.0, 3.0),
            block: Block::from_state_id(1),
        });
        handler.on_interact(&mut BlockInteractContext {
            instance: Entity::PLACEHOLDER,
            position: Position::new(1.0, 2.0, 3.0),
            block: Block::from_state_id(1),
            player: Entity::PLACEHOLDER,
        });
        assert!(handlers.is_empty());
    }

    #[test]
    fn registered_handler_receives_break_callback() {
        let broken = Arc::new(AtomicBool::new(false));
        let handler: Arc<dyn BlockHandler> = Arc::new(FlagHandler {
            broken: Arc::clone(&broken),
        });
        let mut handlers = BlockHandlers::new();
        handlers.register_by_id(2, handler);

        let queried = handlers.handler_by_id(2);
        assert!(queried.is_some());
        queried.unwrap().on_break(&mut BlockBreakContext {
            instance: Entity::PLACEHOLDER,
            position: Position::new(0.0, 0.0, 0.0),
            block: Block::from_state_id(2),
        });
        assert!(broken.load(Ordering::SeqCst));
    }

    #[test]
    fn sparse_vec_does_not_contain_gaps_as_registered() {
        // 注册 id=100 的处理器后，len() 应仍为 1（只计 Some 槽位）。
        let mut handlers = BlockHandlers::new();
        handlers.register_by_id(100, Arc::new(FlagHandler {
            broken: Arc::new(AtomicBool::new(false)),
        }));
        assert_eq!(handlers.len(), 1);
        assert!(!handlers.is_empty());
        assert!(handlers.handler_by_id(100).is_some());
        assert!(handlers.handler_by_id(99).is_none());
    }
}
