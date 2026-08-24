//! 测试辅助：构建「实例 World」测试环境（R11 / T12.5）。
//!
//! 生产代码经 [`InstanceScheduler`](particlemc_framework_ecs::scheduler::InstanceScheduler)
//! 托管多个实例 World：实体落于实例 World、区块数据存于其实例
//! [`ChunkStore`](crate::instance::chunk_store::ChunkStore)；主 World 仅持有全局
//! 系统与（本辅助注入的）`InstanceScheduler` 资源。本模块为测试提供一致的
//! 「主 World（插件装配）+ 实例 World（含 `ChunkStore` / `Shared` 注册表）」装配
//! 入口，使 `chunk_send` / `entity_sync` / `inventory_sync` 等跨 World 系统可在
//! 测试中按生产路径驱动。
#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use particlemc_framework_ecs::scheduler::{InstanceScheduler, WorldId};

use crate::app::App;
use crate::instance::chunk_store::ChunkStore;
use crate::resource::instance_manager::{InstanceManager, SharedRegistries, build_instance_world};
use crate::resource::registries::{
    BlockRegistry, EntityTypeRegistry, GenericRegistry, ItemRegistry,
};

/// 从主 World 已注入的注册表克隆出 [`SharedRegistries`]，供实例 World 零拷贝注入。
///
/// 与 `server/src/lib.rs` 生产装配同构：取主 World 注册表 `clone()` 后包 `Arc`。
pub fn shared_from_app(app: &App) -> SharedRegistries {
    let block = app
        .world()
        .resource::<BlockRegistry>()
        .expect("BlockRegistry 未注入主 World")
        .clone();
    let item = app
        .world()
        .resource::<ItemRegistry>()
        .expect("ItemRegistry 未注入主 World")
        .clone();
    let entity_type = app
        .world()
        .resource::<EntityTypeRegistry>()
        .expect("EntityTypeRegistry 未注入主 World")
        .clone();
    let generic = app
        .world()
        .resource::<GenericRegistry>()
        .expect("GenericRegistry 未注入主 World")
        .clone();
    SharedRegistries {
        block: Arc::new(block),
        item: Arc::new(item),
        entity_type: Arc::new(entity_type),
        generic: Arc::new(generic),
    }
}

/// 在主 World 装配一个真实实例 World 并注入 `InstanceScheduler` 资源。
///
/// - 从主 World 注册表克隆 [`SharedRegistries`](crate::resource::instance_manager::SharedRegistries)；
/// - 经 [`build_instance_world`] 注册实例 World（含 `ChunkStore` 与共享注册表）；
/// - `chunk_setup` 回调加载区块到该实例 World 的 `ChunkStore`；
/// - 将 `InstanceScheduler` 作为资源注入主 World（供跨 World 系统读取）。
///
/// 返回实例 World 的 [`WorldId`]，供后续 [`spawn_into_instance`] 与 `InstanceRef` 使用。
pub fn build_test_instance(app: &mut App, chunk_setup: impl FnOnce(&mut ChunkStore)) -> WorldId {
    let shared = shared_from_app(app);
    let mut scheduler = InstanceScheduler::default();
    let inst = build_instance_world(&mut scheduler, None, None, &shared);
    {
        let mut guard = scheduler.lock_world(inst).expect("实例 World 已注册");
        let store = guard
            .world()
            .resource_mut::<ChunkStore>()
            .expect("实例 World 已注入 ChunkStore");
        chunk_setup(store);
    }
    app.insert_resource(scheduler);
    inst
}

/// 向指定实例 World 生成实体（经 `scheduler.lock_world` 跨 World spawn）。
///
/// 返回实体在该实例 World 的 `Entity` 句柄；主 World 的 `ConnectionManager`
/// 可用同一句柄绑定 conn（实体 id 在同一实例 World 内有效）。
pub fn spawn_into_instance(
    app: &mut App,
    inst: WorldId,
    bundle: impl particlemc_framework_ecs::world::Bundle,
) -> crate::prelude::Entity {
    let sched = app
        .world_mut()
        .resource_mut::<InstanceScheduler>()
        .expect("InstanceScheduler 未注入主 World");
    let mut guard = sched.lock_world(inst).expect("实例 World 已注册");
    guard.world().spawn_bundle(bundle).id()
}

/// 缓存惰性测试实例的 `WorldId`（仅测试内使用）。
///
/// 同一 App 的多个玩家须落入同一实例 World：否则各 World 首个 spawn 的
/// `Entity` id 可能同为 `(0,0)`，导致 `ConnectionManager` 与跨 World 收集
/// 逻辑以 entity id 为键时发生冲突。
pub struct TestInstanceId(pub u32);

/// 惰性获取（或创建）测试实例 World：同一 App 多次调用复用同一实例。
///
/// 与 `build_test_instance` 不同，本函数把首次创建的 `WorldId` 缓存进主 World
/// 资源，后续调用直接返回，从而保证多名玩家共享同一实例 World（实体 id 不冲突）。
pub fn ensure_test_instance(app: &mut App) -> WorldId {
    if let Some(id) = app.world().get_resource::<TestInstanceId>() {
        return WorldId(id.0);
    }
    let inst = build_test_instance(app, |_store| {});
    let wid = inst.0;
    app.world_mut().insert_resource(TestInstanceId(wid));
    inst
}

/// 同 [`ensure_test_instance`]，但额外注入 `InstanceManager` 并将其默认实例
/// 指向测试实例 World。供需要从 `on_join` 经 `instance_mgr.default_instance()`
/// 落子到实例 World 的测试（如 `network_receive` 的点击 / 关窗 / 切手持槽）。
pub fn ensure_test_instance_default(app: &mut App) -> WorldId {
    let inst = ensure_test_instance(app);
    let mut mgr = InstanceManager::default();
    mgr.set_default(inst);
    app.insert_resource(mgr);
    inst
}

/// 读取已缓存的惰性测试实例 WorldId（须先经 [`ensure_test_instance`] 创建，
/// 例如通过 `spawn_player` / `spawn_player_with_inventory`）。
pub fn current_test_instance(app: &App) -> WorldId {
    let id = app
        .world()
        .get_resource::<TestInstanceId>()
        .expect("测试实例未初始化：请先调用 ensure_test_instance / spawn_player / spawn_player_with_inventory")
        .0;
    WorldId(id)
}

/// 在指定实例 World 内对实体取可变组件并执行操作（跨 World 取组件安全封装）。
///
/// 实体已迁入实例 World，主 World 的 `get_mut` 取不到；本函数经 `scheduler`
/// 跨 World 锁定后取组件，供测试修改实例内实体的组件（如 `PlayerInventory`）。
pub fn with_instance_entity<T: particlemc_framework_ecs::component::Component, F>(
    app: &mut App,
    inst: WorldId,
    entity: crate::prelude::Entity,
    f: F,
) where
    F: FnOnce(&mut T),
{
    let sched = app
        .world_mut()
        .resource_mut::<InstanceScheduler>()
        .expect("InstanceScheduler 未注入主 World");
    if let Some(mut guard) = sched.lock_world(inst)
        && let Some(c) = guard.world().get_mut::<T>(entity)
    {
        f(&mut *c);
    }
}

/// 跨 World 读取实例内实体的单个组件（克隆返回）。
///
/// 实体已迁入实例 World，主 World 的 `get` 取不到；本函数经 `scheduler` 跨 World
/// 锁定后取组件并克隆，供测试只读断言（如 `Position`）。组件须实现 `Clone`。
pub fn read_instance<T: particlemc_framework_ecs::component::Component + Clone>(
    app: &mut App,
    inst: WorldId,
    entity: crate::prelude::Entity,
) -> Option<T> {
    let sched = app
        .world_mut()
        .resource_mut::<InstanceScheduler>()
        .expect("InstanceScheduler 未注入主 World");
    let mut guard = sched.lock_world(inst)?;
    guard.world().get::<T>(entity).cloned()
}
