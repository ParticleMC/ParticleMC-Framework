// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 集成测试：proc-macro 在外部 crate（stub 模拟 `particlemc_framework_ecs` 权威类型）中的展开。
//!
//! 变更标识符：`implement-custom-ecs`
//!
//! 顶层 `particlemc_framework_ecs` 模块树与真实 crate 签名一致（T1 `component.rs` 的
//! `Component` trait / `register_component_id`，IC-1 `EntityTypeId`，IC-3
//! `ArchetypeId` / `ArchetypeDef`，IC-8 `Message`），用于验证宏输出可在外部
//! crate 编译且语义正确。真实 `particlemc-framework-ecs` crate 的 `archetype` / `message`
//! 模块由后续任务（T3/T6）实现，宏按此签名生成。

// 夹具结构体仅作为宏输入与类型断言使用，字段值不参与断言，避免 dead_code 噪音。
#![allow(dead_code)]
// 测试代码允许 unwrap/expect（章程豁免条款；生产代码仍受 clippy 门禁约束）。
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod particlemc_framework_ecs {
    pub mod component {
        use std::any::TypeId;
        use std::sync::{Mutex, OnceLock};

        /// 组件存储类别（与 T1 `component.rs` 一致）。
        #[derive(Copy, Clone, PartialEq, Eq, Debug)]
        pub enum ComponentStorage {
            SoA,
            Sparse,
        }

        /// 全局唯一组件标识。
        #[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
        pub struct ComponentId(pub u16);

        /// 组件注册契约（IC-2）。
        pub trait Component: Sized + 'static {
            fn id() -> ComponentId;
            const STORAGE: ComponentStorage;
            type Registry;
        }

        /// 与真实实现行为一致：同一 TypeId 幂等，不同 TypeId 按序递增分配。
        pub fn register_component_id(type_id: TypeId) -> ComponentId {
            static REGISTRY: OnceLock<Mutex<Vec<TypeId>>> = OnceLock::new();
            let mut table = REGISTRY
                .get_or_init(|| Mutex::new(Vec::new()))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(index) = table.iter().position(|entry| *entry == type_id) {
                return ComponentId(to_u16(index));
            }
            table.push(type_id);
            ComponentId(to_u16(table.len() - 1))
        }

        fn to_u16(index: usize) -> u16 {
            u16::try_from(index).unwrap_or(u16::MAX)
        }
    }

    pub mod entity {
        /// 实体类型 ID（IC-1）。
        #[derive(Copy, Clone, PartialEq, Eq, Debug)]
        pub struct EntityTypeId(pub u8);
    }

    pub mod archetype {
        use super::component::ComponentId;
        use super::entity::EntityTypeId;

        /// Archetype ID（IC-3）。
        #[derive(Copy, Clone, PartialEq, Eq, Debug)]
        pub struct ArchetypeId(pub u16);

        /// Archetype 运行时定义（IC-3；含 `component_types`，Query 匹配使用）。
        pub struct ArchetypeDef {
            pub id: ArchetypeId,
            pub name: &'static str,
            pub component_ids: &'static [ComponentId],
            pub entity_kind: EntityTypeId,
            pub component_types: &'static [std::any::TypeId],
        }
    }

    pub mod message {
        /// 消息契约（IC-8）。
        pub trait Message: Send + Sync + 'static {}
    }
}

use particlemc_framework_ecs::component::{Component, ComponentStorage};

// ---- 样例组件 ----

#[derive(particlemc_framework_ecs_macros::Component)]
struct Position {
    x: f32,
    y: f32,
}

#[derive(particlemc_framework_ecs_macros::Component)]
struct Velocity {
    dx: f32,
    dy: f32,
}

#[derive(particlemc_framework_ecs_macros::Component)]
#[component(storage = "sparse")]
struct Potion {
    level: u8,
}

#[derive(particlemc_framework_ecs_macros::Component)]
#[component(storage = "soa")]
struct Health {
    hp: f32,
}

/// 泛型组件：自动追加 `T: 'static` 约束。
#[derive(particlemc_framework_ecs_macros::Component)]
struct GenericMarker<T> {
    value: T,
}

// ---- 样例 Archetype ----

#[derive(particlemc_framework_ecs_macros::Archetype)]
struct PlayerArchetype {
    position: Position,
    velocity: Velocity,
    health: Health,
    player_marker: GenericMarker<u8>,
}

#[derive(particlemc_framework_ecs_macros::Archetype)]
struct ItemArchetype {
    position: Position,
}

// ---- 样例 Message ----

#[derive(particlemc_framework_ecs_macros::Message)]
struct PlayerJoin {
    name: String,
}

/// 泛型消息：自动追加 `T: Send + Sync + 'static` 约束。
#[derive(particlemc_framework_ecs_macros::Message)]
struct BlockEvent<T> {
    payload: T,
}

// ---- 注册宏 ----

particlemc_framework_ecs_macros::register_archetypes! {
    PlayerArchetype => EntityTypeId(0),
    /// 物品实体 Archetype（实体类型 1）
    ItemArchetype => 1,
}

particlemc_framework_ecs_macros::register_components!(Position, Velocity, Potion, Health);

// ---- 测试断言 ----

#[test]
fn component_id_is_idempotent_and_incrementing() {
    let first = <Position as Component>::id();
    assert_eq!(first, <Position as Component>::id());
    let second = <Velocity as Component>::id();
    assert_ne!(first, second);
    assert!(second.0 > first.0);
}

#[test]
fn storage_attribute_controls_component_storage() {
    assert_eq!(<Position as Component>::STORAGE, ComponentStorage::SoA);
    assert_eq!(<Health as Component>::STORAGE, ComponentStorage::SoA);
    assert_eq!(<Potion as Component>::STORAGE, ComponentStorage::Sparse);
}

#[test]
fn archetype_constants_are_generated() {
    assert_eq!(PLAYER_ARCHETYPE, particlemc_framework_ecs::archetype::ArchetypeId(0));
    assert_eq!(ITEM_ARCHETYPE, particlemc_framework_ecs::archetype::ArchetypeId(1));
}

#[test]
fn archetype_table_is_populated_in_order() {
    assert_eq!(ARCHETYPES.len(), 2);

    let player = ARCHETYPES.first().unwrap();
    assert_eq!(player.id, particlemc_framework_ecs::archetype::ArchetypeId(0));
    assert_eq!(player.name, "PlayerArchetype");
    assert_eq!(player.entity_kind.0, 0);
    assert_eq!(player.component_ids.len(), 4);
    assert_eq!(player.component_types.len(), player.component_ids.len());
    // 字段序即组件注册序：首个组件 ID 与 Position 一致，类型与 TypeId 一致
    assert_eq!(
        player.component_ids.first().copied(),
        Some(<Position as Component>::id())
    );
    assert_eq!(
        player.component_types.first().copied(),
        Some(std::any::TypeId::of::<Position>())
    );

    let item = ARCHETYPES.get(1).unwrap();
    assert_eq!(item.id, particlemc_framework_ecs::archetype::ArchetypeId(1));
    assert_eq!(item.name, "ItemArchetype");
    assert_eq!(item.entity_kind.0, 1);
    assert_eq!(item.component_ids.len(), 1);
}

#[test]
fn component_list_matches_field_types() {
    let _: PlayerArchetypeComponentList = (
        Position { x: 0.0, y: 0.0 },
        Velocity { dx: 0.0, dy: 0.0 },
        Health { hp: 1.0 },
        GenericMarker { value: 1u8 },
    );
    let _: ItemArchetypeComponentList = (Position { x: 0.0, y: 0.0 },);
}

#[test]
fn register_functions_compile_and_run() {
    __register_all_components();
    register_all();
    // 注册后组件 ID 稳定（幂等）
    assert_eq!(<Position as Component>::id(), <Position as Component>::id());
}

#[test]
fn message_derive_impls_trait() {
    fn assert_message<T: particlemc_framework_ecs::message::Message>() {}
    assert_message::<PlayerJoin>();
    assert_message::<BlockEvent<String>>();
}
