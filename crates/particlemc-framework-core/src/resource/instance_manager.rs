//! 实例（世界）管理器：维护默认实例与已注册实例表（R11）。
//!
//! 以 `Uuid` 为键登记各个 `Instance` 对应的 [`WorldId`]，并持有「默认实例」
//! 标识。框架不生成任何世界内容（出生平台等），世界由应用侧通过
//! [`build_instance_world`] + 本管理器装配。区块 / 生成器 / 持久化器作为
//! `ChunkStore` 存于实例 World 内（见 `crate::instance::ChunkStore`）。

use std::collections::HashMap;
use std::sync::Arc;

use particlemc_framework_ecs::scheduler::{InstanceScheduler, WorldId};
use uuid::Uuid;

use crate::instance::{ChunkGenerator, ChunkLoader, ChunkStore};
use crate::prelude::{Schedule, World};
use crate::resource::registries::{
    BlockRegistry, EntityTypeRegistry, GenericRegistry, ItemRegistry,
};
use crate::system::{entity_ai, physics, player_movement};

/// 实例管理器：默认实例 + 已注册实例表（Uuid ↔ WorldId）。
#[derive(Default)]
pub struct InstanceManager {
    /// Uuid → 实例 WorldId。
    by_uuid: HashMap<Uuid, WorldId>,
    /// WorldId → Uuid（反向索引）。
    by_world: HashMap<WorldId, Uuid>,
    /// 默认实例 WorldId（由应用侧装配，可为空）。
    default_instance: Option<WorldId>,
}

impl InstanceManager {
    /// 登记一个实例（Uuid ↔ WorldId 双向映射），返回被替换的旧 WorldId（若有）。
    pub fn register_uuid(&mut self, uuid: Uuid, world_id: WorldId) -> Option<WorldId> {
        let old = self.by_uuid.insert(uuid, world_id);
        if let Some(old_id) = old {
            self.by_world.remove(&old_id);
        }
        self.by_world.insert(world_id, uuid);
        old
    }

    /// 注销一个实例，返回其 Uuid（若有）。
    pub fn unregister(&mut self, world_id: WorldId) -> Option<Uuid> {
        if let Some(uuid) = self.by_world.remove(&world_id) {
            self.by_uuid.remove(&uuid);
            if self.default_instance == Some(world_id) {
                self.default_instance = None;
            }
            Some(uuid)
        } else {
            None
        }
    }

    /// 按 Uuid 查找实例对应的 WorldId。
    pub fn world_id_of(&self, uuid: &Uuid) -> Option<WorldId> {
        self.by_uuid.get(uuid).copied()
    }

    /// 按 WorldId 查找实例对应的 Uuid。
    pub fn uuid_of(&self, world_id: WorldId) -> Option<Uuid> {
        self.by_world.get(&world_id).copied()
    }

    /// 已注册实例数量。
    pub fn len(&self) -> usize {
        self.by_uuid.len()
    }

    /// 是否未注册任何实例。
    pub fn is_empty(&self) -> bool {
        self.by_uuid.is_empty()
    }

    /// 返回默认实例 WorldId。
    pub fn default_instance(&self) -> Option<WorldId> {
        self.default_instance
    }

    /// 设定默认实例 WorldId。
    pub fn set_default(&mut self, world_id: WorldId) {
        self.default_instance = Some(world_id);
    }
}

/// 跨实例共享的只读注册表集合（R11.5 / 12.5）。
///
/// 每个字段为 `Arc<T>`，由主 World 拥有的注册表克隆出 `Arc` 值注入各实例 World，
/// 经 [`World::insert_shared`] 形成 [`particlemc_framework_ecs::shared::Shared<T>`]，实现多实例
/// World 零拷贝只读共享。4 个具名注册表均 `#[derive(Clone)]`，可安全
/// `Arc::new(value.clone())`。
#[derive(Clone)]
pub struct SharedRegistries {
    /// 方块注册表（只读共享）。
    pub block: Arc<BlockRegistry>,
    /// 物品注册表（只读共享）。
    pub item: Arc<ItemRegistry>,
    /// 实体类型注册表（只读共享）。
    pub entity_type: Arc<EntityTypeRegistry>,
    /// 通用变体类注册表（只读共享）。
    pub generic: Arc<GenericRegistry>,
}

/// 构建并注册一个实例 World，返回分配的 [`WorldId`]（R11）。
///
/// - 新建 `World`，注入 `ChunkStore`（含生成器 / 持久化器）作为实例区块存储；
/// - 经 [`Shared<T>`](particlemc_framework_ecs::shared::Shared) 零拷贝注入 4 个只读注册表
///   （R11.5 / 12.5），供实例内系统（physics 等）只读访问，无需跨 World 回主
///   World 取；
/// - 实例 `Schedule` 装配实体相关系统（R11.2 实体迁入后）：`player_movement` →
///   `entity_ai` → `physics`（依赖顺序同主 World 管线），实体与系统同步落入
///   本实例 World；注册进 `scheduler` 后由其 `tick_all` 并行驱动。
pub fn build_instance_world(
    scheduler: &mut InstanceScheduler,
    generator: Option<Box<dyn ChunkGenerator>>,
    loader: Option<Box<dyn ChunkLoader>>,
    shared: &SharedRegistries,
) -> WorldId {
    let mut world = World::new();
    let mut store = ChunkStore::new();
    if let Some(r#gen) = generator {
        store.set_generator(r#gen);
    }
    if let Some(ld) = loader {
        store.set_loader(ld);
    }
    world.insert_resource(store);
    // R11.5 / 12.5 零拷贝：注入共享只读注册表（各实例 World 持有同一 Arc）。
    world.insert_shared(shared.block.clone());
    world.insert_shared(shared.item.clone());
    world.insert_shared(shared.entity_type.clone());
    world.insert_shared(shared.generic.clone());
    let mut schedule = Schedule::new();
    // R11.2：实体迁入实例 World，对应系统同步迁入（依赖顺序：物理在 AI 之后）。
    // 此处直接调用 `Schedule::after`（接受 `impl SystemLabel`，仅 `&str`/`String`
    // 实现），故用与 `App::after` 内部 `type_name_of_val` 一致的「函数路径」字符串
    // 作为标签，精确关联到 `add_system` 注册的同名系统
    // （`type_name::<F>()` = `particlemc_framework_core::system::<mod>::<fn>`）。
    schedule
        .add_system(player_movement)
        .add_system(entity_ai)
        .add_system(physics)
        .after(
            "particlemc_framework_core::system::entity_ai::entity_ai",
            "particlemc_framework_core::system::player_movement::player_movement",
        )
        .after(
            "particlemc_framework_core::system::physics::physics",
            "particlemc_framework_core::system::entity_ai::entity_ai",
        );
    scheduler.register_new_world(world, schedule)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn register_and_lookup_roundtrip() {
        let mut mgr = InstanceManager::default();
        let uuid = Uuid::nil();
        mgr.register_uuid(uuid, WorldId(5));
        assert_eq!(mgr.world_id_of(&uuid), Some(WorldId(5)));
        assert_eq!(mgr.uuid_of(WorldId(5)), Some(uuid));
        assert_eq!(mgr.unregister(WorldId(5)), Some(uuid));
        assert!(mgr.is_empty());
    }

    #[test]
    fn set_default_tracks_default_instance() {
        let mut mgr = InstanceManager::default();
        assert!(mgr.default_instance().is_none());
        mgr.set_default(WorldId(9));
        assert_eq!(mgr.default_instance(), Some(WorldId(9)));
    }
}
