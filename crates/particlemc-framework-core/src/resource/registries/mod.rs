// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 注册表层模块：核心 [`Registry`] 与各类具名注册表。
//!
//! 这里集中导出方块、物品、实体类型等具名注册表，以及承接其余变体类与标签的
//! [`GenericRegistry`] / [`TagRegistry`]。所有注册表均实现 旧 ECS 方案 `Resource`，
//! 由 [`crate::plugin::McServerPlugin`] 在启动时注入 `World`。
//!
//! 运行时覆盖能力：`Registry<T>` 提供 [`Registry::override_value`] /
//! [`Registry::register_or_replace`]，可对已加载注册数据做运行时替换
//! （保留原 id），各具名注册表（`BlockRegistry` / `ItemRegistry` /
//! `EntityTypeRegistry` / `GenericRegistry`）均转发暴露。

pub mod block;
pub mod entity_type;
pub mod generic;
pub mod item;
pub mod loot;
pub mod nbt;
pub mod registry;
pub mod tags;
pub mod world;

pub use block::BlockRegistry;
pub use entity_type::EntityTypeRegistry;
pub use generic::GenericRegistry;
pub use item::ItemRegistry;
pub use loot::{LootTable, LootTableRegistry};
pub use nbt::{RegistrySnapshot, registry_data_packets, update_tags_packet};
pub use registry::{
    BlockDefinition, BlockStateDef, EntityTypeDefinition, GenericDefinition, ItemDefinition,
    Registry, RegistryEntry, RegistryError,
};
pub use tags::TagRegistry;
pub use world::{
    BiomeRegistry, DimensionTypeRegistry, EnchantmentRegistry, FluidRegistry, ParticleRegistry,
    PotionEffectRegistry, SoundEventRegistry,
};
