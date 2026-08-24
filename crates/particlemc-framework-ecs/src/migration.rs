// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 实体跨世界迁移原语。
//!
//! 变更标识符：`implement-custom-ecs`
//!
//! IC-12：`migrate_entity(src, dst, e)` —— 将实体从源世界移动到目标世界，
//! 组件全量随迁，源世界实体销毁（不可见）。
//!
//! ## U3 合规说明
//!
//! 本模块为**纯 safe 模块**。U3（`ptr::copy_nonoverlapping` 零拷贝迁移）在
//! 本实现中**不产生 unsafe**：迁移以"值 move + 所有权转移"实现——组件值经
//! [`crate::storage::ErasedColumn::take_at`/`take_slot`] 以
//! `Box<dyn Any + Send + Sync>` 所有权移出源列，实体经 `World::despawn` 从
//! 源世界销毁，再经 `World::spawn_exact`/`World::insert_any` 写入目标列。
//! 跨世界必然跨内存（各世界列独立分配），值语义 = 零序列化迁移（无需
//! `Copy`/`Serialize` 约束）。U3 所表达的能力已由所有权转移覆盖，故不引入
//! 裸指针。
//!
//! ## 前置条件（调用方保证，违反返回 [`MigrateError::DifferentKind`]）
//!
//! 目标世界 `dst` 必须：
//! 1. 已注册与源实体相同的 `ArchetypeId`；
//! 2. 该 Archetype 的组件列已就绪——即对目标 Archetype 至少执行过一次
//!    `insert<T>`（SoA 列惰性创建，`insert_any` 仅支持列已存在）。迁移前
//!    未建列的组件写入将返回 `Err`，此时 dst 中实体已部分迁移（调用方应
//!    处理部分失败：despawn 残留实体或修复列后重试）。
//!
//! ## ID 保留语义
//!
//! 优先保留源实体 ID（经 `World::spawn_exact` → `EntityArena::allocate_exact`）：
//! 目标世界该槽位空闲则原样落位（ID 不变）；被占用则重新分配新 ID（返回
//! 实际 `Entity`，**目标槽位冲突时 ID 变化**）。

use std::any::Any;

use crate::archetype::ArchetypeId;
use crate::component::ComponentId;
use crate::entity::Entity;
use crate::world::World;

/// 实体迁移错误（IC-12 冻结契约，三变体）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrateError {
    /// 源世界不存在该实体（从未 spawn、已销毁或悬挂句柄）。
    EntityNotFound,
    /// 实体已在目标世界（先前迁移成功，源已销毁）。
    AlreadyMigrated,
    /// 目标世界未注册源实体的 Archetype，或实体类型与目标 Archetype 的
    /// `entity_kind` 不一致；亦用于目标组件列未就绪（前置条件被破坏）。
    DifferentKind,
}

/// 将实体从源世界迁移到目标世界（IC-12）。
///
/// 流程：校验源/目标 → 按值收集组件 → `src.despawn` → `dst.spawn_exact`
/// （优先保留 ID）→ 逐组件 `dst.insert_any` 写入。
///
/// # Errors
///
/// - 源无该实体：若实体已在目标世界（先前迁移）返回
///   [`MigrateError::AlreadyMigrated`]，否则 [`MigrateError::EntityNotFound`]。
/// - 目标世界未注册源 Archetype / 实体类型不匹配 / 目标组件列未就绪 →
///   [`MigrateError::DifferentKind`]。注意：列未就绪的失败发生在源实体已
///   despawn、目标实体已落位之后（部分迁移），调用方负责清理。
pub fn migrate_entity(src: &mut World, dst: &mut World, e: Entity) -> Result<Entity, MigrateError> {
    // 1. 源实体存在性
    if !src.contains(e) {
        // 已迁移：实体从源销毁且存在于目标 → AlreadyMigrated；否则从未存在
        return if dst.contains(e) {
            Err(MigrateError::AlreadyMigrated)
        } else {
            Err(MigrateError::EntityNotFound)
        };
    }
    // 2. 源 Archetype 与槽位索引
    let arch = match src.entity_archetype(e) {
        Some(arch) => arch,
        // 不可达：contains 已验证存活，entity_index 必含该实体
        None => return Err(MigrateError::EntityNotFound),
    };
    let idx = match src.entity_index.get(&e) {
        Some(&(_, idx)) => idx,
        // 不可达：同上
        None => return Err(MigrateError::EntityNotFound),
    };
    // 3. 目标世界须已注册同一 Archetype 且实体类型一致（DifferentKind）
    let dst_def = match dst.archetypes.get(&arch) {
        Some(storage) => storage.def,
        None => return Err(MigrateError::DifferentKind),
    };
    if dst_def.entity_kind != e.type_id() {
        return Err(MigrateError::DifferentKind);
    }
    // 4. 收集组件值：所有权移出源列（SoA 取走并重置默认，Sparse 移出值）
    let values = collect_components(src, e, arch, idx);
    // 5. 源世界销毁实体（防重复迁移：此后 src 不含 e）
    src.despawn(e);
    // 6. 目标世界落位（优先保留 ID）
    let entity = dst.spawn_exact(arch, e);
    // 7. 逐组件写入目标列（前置条件保证列已就绪，失败视为调用方违约）
    for (cid, value) in values {
        if dst.insert_any(entity, cid, value).is_err() {
            return Err(MigrateError::DifferentKind);
        }
    }
    Ok(entity)
}

/// 按值收集实体的全部组件，返回 `(ComponentId, 类型擦除值)` 列表。
///
/// 遍历该 Archetype 的**全部**列（含不在 `def.component_ids` 中的 Sparse 列）：
/// SoA 列经 `take_at`（Archetype 槽位索引），Sparse 列经 `take_slot`（实体
/// 槽位）；无值（Sparse 未 insert 过）跳过。
fn collect_components(
    src: &mut World,
    e: Entity,
    arch: ArchetypeId,
    idx: usize,
) -> Vec<(ComponentId, Box<dyn Any + Send + Sync>)> {
    let mut values = Vec::new();
    let storage = match src.archetypes.get_mut(&arch) {
        Some(storage) => storage,
        // 不可达：migrate_entity 第 2 步已校验 arch 存在
        None => unreachable!("源 Archetype 已由 entity_archetype 校验存在"),
    };
    for (cid, column) in storage.columns.iter_mut() {
        let value = if column.is_sparse() {
            column.take_slot(e.slot().0)
        } else {
            column.take_at(idx)
        };
        if let Some(value) = value {
            values.push((*cid, value));
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archetype::ArchetypeDef;
    use crate::component::{Component, ComponentStorage};
    use crate::entity::{EntityTypeId, Generation, Slot};

    // ---- 测试组件（手工实现 Component，避免测试依赖宏 crate）----

    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    struct Position {
        x: f32,
        y: f32,
    }

    impl Component for Position {
        fn id() -> ComponentId {
            ComponentId(1)
        }
        const STORAGE: ComponentStorage = ComponentStorage::SoA;
        type Registry = ();
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    struct Velocity {
        dx: f32,
        dy: f32,
    }

    impl Component for Velocity {
        fn id() -> ComponentId {
            ComponentId(2)
        }
        const STORAGE: ComponentStorage = ComponentStorage::SoA;
        type Registry = ();
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    struct Health {
        hp: u32,
    }

    impl Component for Health {
        fn id() -> ComponentId {
            ComponentId(3)
        }
        const STORAGE: ComponentStorage = ComponentStorage::Sparse;
        type Registry = ();
    }

    /// 玩家 Archetype：SoA Position + Velocity + Sparse Health，实体类型 1。
    static PLAYER_DEF: ArchetypeDef = ArchetypeDef {
        id: ArchetypeId(0),
        name: "PlayerArchetype",
        component_ids: &[ComponentId(1), ComponentId(2)],
        entity_kind: EntityTypeId(1),
        component_types: &[],
    };

    /// 同 id 0 但实体类型不同的冲突定义（目标侧误注册场景）。
    static ALIEN_DEF: ArchetypeDef = ArchetypeDef {
        id: ArchetypeId(0),
        name: "AlienArchetype",
        component_ids: &[ComponentId(1), ComponentId(2)],
        entity_kind: EntityTypeId(2),
        component_types: &[],
    };

    /// 在目标世界建好 PLAYER 全部组件列后销毁种子实体（腾出槽位 0），
    /// 使迁移可保留源 ID。返回值丢弃：种子实体已 despawn。
    fn seed_columns_then_free_slot(dst: &mut World) {
        let seed = dst.spawn(ArchetypeId(0));
        dst.insert(seed, Position::default()).unwrap();
        dst.insert(seed, Velocity::default()).unwrap();
        dst.insert(seed, Health { hp: 1 }).unwrap();
        assert!(dst.despawn(seed));
    }

    /// 构建 src/dst 各注册 PLAYER 的迁移环境，并在 src 生成带全部组件的实体。
    fn setup() -> (World, World, Entity) {
        let mut src = World::new();
        let mut dst = World::new();
        src.register_archetype(&PLAYER_DEF);
        dst.register_archetype(&PLAYER_DEF);
        seed_columns_then_free_slot(&mut dst);
        let e = src.spawn(ArchetypeId(0)); // src 槽位 0
        src.insert(e, Position { x: 1.0, y: 2.0 }).unwrap();
        src.insert(e, Velocity { dx: 3.0, dy: 4.0 }).unwrap();
        src.insert(e, Health { hp: 99 }).unwrap();
        (src, dst, e)
    }

    #[test]
    fn migrate_preserves_id_and_all_components() {
        let (mut src, mut dst, e) = setup();
        let migrated = migrate_entity(&mut src, &mut dst, e).unwrap();
        // ID 保留（目标槽位空闲）
        assert_eq!(migrated, e);
        // 源世界不可见，目标世界全组件可见（SoA + Sparse 全量）
        assert!(!src.contains(e));
        assert!(dst.contains(migrated));
        assert_eq!(
            dst.get::<Position>(migrated),
            Some(&Position { x: 1.0, y: 2.0 })
        );
        assert_eq!(
            dst.get::<Velocity>(migrated),
            Some(&Velocity { dx: 3.0, dy: 4.0 })
        );
        assert_eq!(dst.get::<Health>(migrated), Some(&Health { hp: 99 }));
        assert_eq!(src.entity_count(), 0);
        assert_eq!(dst.entity_count(), 1);
    }

    #[test]
    fn migrate_twice_returns_already_migrated() {
        let (mut src, mut dst, e) = setup();
        let migrated = migrate_entity(&mut src, &mut dst, e).unwrap();
        assert_eq!(migrated, e);
        // 再次迁移：源无 e、目标有 e → AlreadyMigrated
        assert_eq!(
            migrate_entity(&mut src, &mut dst, e),
            Err(MigrateError::AlreadyMigrated)
        );
    }

    #[test]
    fn migrate_unknown_entity_returns_not_found() {
        let (mut src, mut dst, _e) = setup();
        let ghost = Entity::from_parts(EntityTypeId(1), Generation(0), Slot(999));
        assert_eq!(
            migrate_entity(&mut src, &mut dst, ghost),
            Err(MigrateError::EntityNotFound)
        );
    }

    #[test]
    fn migrate_after_src_despawn_returns_not_found() {
        let (mut src, mut dst, e) = setup();
        // 源直接销毁（非迁移）：src 无 e 且 dst 无 e → EntityNotFound
        assert!(src.despawn(e));
        assert_eq!(
            migrate_entity(&mut src, &mut dst, e),
            Err(MigrateError::EntityNotFound)
        );
    }

    #[test]
    fn migrate_unregistered_dst_arch_returns_different_kind() {
        let mut src = World::new();
        let mut dst = World::new();
        src.register_archetype(&PLAYER_DEF);
        // dst 未注册任何 Archetype
        let e = src.spawn(ArchetypeId(0));
        assert_eq!(
            migrate_entity(&mut src, &mut dst, e),
            Err(MigrateError::DifferentKind)
        );
        // 无副作用：源实体保留，目标为空
        assert!(src.contains(e));
        assert_eq!(dst.entity_count(), 0);
    }

    #[test]
    fn migrate_entity_kind_mismatch_returns_different_kind() {
        let mut src = World::new();
        let mut dst = World::new();
        src.register_archetype(&PLAYER_DEF);
        // 目标侧注册同 id 0 但实体类型不同的 ALIEN_DEF（误配置）
        dst.register_archetype(&ALIEN_DEF);
        let e = src.spawn(ArchetypeId(0)); // kind 1
        assert_eq!(
            migrate_entity(&mut src, &mut dst, e),
            Err(MigrateError::DifferentKind)
        );
        assert!(src.contains(e));
        assert_eq!(dst.entity_count(), 0);
    }

    #[test]
    fn migrate_slot_conflict_reallocates_id() {
        let mut src = World::new();
        let mut dst = World::new();
        src.register_archetype(&PLAYER_DEF);
        dst.register_archetype(&PLAYER_DEF);
        // dst 槽位 0 被种子实体占用（不 despawn），迁移将重分配 ID
        let _seed = dst.spawn(ArchetypeId(0));
        dst.insert(_seed, Position::default()).unwrap();
        dst.insert(_seed, Velocity::default()).unwrap();
        dst.insert(_seed, Health { hp: 1 }).unwrap();
        let e = src.spawn(ArchetypeId(0)); // src 槽位 0
        src.insert(e, Position { x: 5.0, y: 6.0 }).unwrap();
        src.insert(e, Velocity { dx: 1.0, dy: 1.0 }).unwrap();
        src.insert(e, Health { hp: 77 }).unwrap();
        let migrated = migrate_entity(&mut src, &mut dst, e).unwrap();
        // 目标槽位冲突 → 返回新 ID（文档语义）
        assert_ne!(migrated, e);
        assert!(!src.contains(e));
        assert!(dst.contains(migrated));
        assert_eq!(
            dst.get::<Position>(migrated),
            Some(&Position { x: 5.0, y: 6.0 })
        );
        assert_eq!(
            dst.get::<Velocity>(migrated),
            Some(&Velocity { dx: 1.0, dy: 1.0 })
        );
        assert_eq!(dst.get::<Health>(migrated), Some(&Health { hp: 77 }));
        // 种子实体未受影响
        assert!(dst.contains(_seed));
        assert_eq!(dst.entity_count(), 2);
    }

    #[test]
    fn migrate_entity_without_components() {
        let mut src = World::new();
        let mut dst = World::new();
        src.register_archetype(&PLAYER_DEF);
        dst.register_archetype(&PLAYER_DEF);
        seed_columns_then_free_slot(&mut dst);
        // 源实体从未 insert 组件（列缺失，无值可迁）
        let e = src.spawn(ArchetypeId(0));
        let migrated = migrate_entity(&mut src, &mut dst, e).unwrap();
        assert_eq!(migrated, e);
        assert!(!src.contains(e));
        assert!(dst.contains(migrated));
        // Position 列已由 seed 建好：迁移实体获得默认占位值（列存在即可见）
        assert_eq!(dst.get::<Position>(migrated), Some(&Position::default()));
        // Health 为 Sparse 且源实体从未 insert：该实体无值
        assert!(dst.get::<Health>(migrated).is_none());
        assert_eq!(dst.entity_count(), 1);
    }

    #[test]
    fn migrate_without_seeded_columns_is_partial_failure() {
        let mut src = World::new();
        let mut dst = World::new();
        src.register_archetype(&PLAYER_DEF);
        dst.register_archetype(&PLAYER_DEF);
        // 目标世界未建列（前置条件被破坏）：组件写入失败
        let e = src.spawn(ArchetypeId(0));
        src.insert(e, Position { x: 1.0, y: 2.0 }).unwrap();
        let result = migrate_entity(&mut src, &mut dst, e);
        assert_eq!(result, Err(MigrateError::DifferentKind));
        // 部分迁移语义：源实体已销毁，目标实体残留（无组件），调用方需清理
        assert!(!src.contains(e));
        assert!(dst.contains(e));
    }

    #[test]
    fn migrate_only_sparse_component_entity() {
        let mut src = World::new();
        let mut dst = World::new();
        src.register_archetype(&PLAYER_DEF);
        dst.register_archetype(&PLAYER_DEF);
        seed_columns_then_free_slot(&mut dst);
        let e = src.spawn(ArchetypeId(0));
        // 仅 Sparse 组件（Health 不在 def.component_ids 中）：验证迁移收集
        // 覆盖全部列而非仅固定 SoA 集合
        src.insert(e, Health { hp: 55 }).unwrap();
        let migrated = migrate_entity(&mut src, &mut dst, e).unwrap();
        assert_eq!(migrated, e);
        assert_eq!(dst.get::<Health>(migrated), Some(&Health { hp: 55 }));
        // Position 列已由 seed 建好：源实体从未 insert，得到默认占位值
        assert_eq!(dst.get::<Position>(migrated), Some(&Position::default()));
    }
}
