//! 组件层：以 旧 ECS 方案 `Component` 形式描述实体数据。
//!
//! 对应 Minestom 的实体属性概念，覆盖坐标、速度、生命值、玩家标识、实例引用、
//! 方块状态与实体类层级（活体 / 生物 / 弹射物 / 掉落物 / 经验球）。这些组件是
//! [`crate::system`] 中 tick 管线的数据载体。实体类层级以**组件组合**替代
//! Java 继承树（`LivingEntity` / `EntityCreature` / `EntityProjectile` /
//! `ItemEntity` / `ExperienceOrb`），见 `living.rs` / `creature.rs` /
//! `projectile.rs` / `item_entity.rs` / `experience_orb.rs`。
//!
//! 变更标识符：`complete-missing-subsystems`（T4 实体类层级）。

pub mod attributes;
pub mod block;
pub mod block_state;
pub mod creature;
pub mod damage;
pub mod digging;
pub mod entity_meta;
pub mod experience_orb;
pub mod health;
pub mod instance_ref;
pub mod inventory;
pub mod item_entity;
pub mod living;
pub mod player;
pub mod position;
pub mod projectile;
pub mod velocity;

pub use attributes::Attributes;
pub use block::Block;
pub use block_state::BlockState;
pub use creature::EntityCreature;
pub use damage::{Damage, DamageSource};
pub use digging::PlayerDiggingState;
pub use entity_meta::{EntityMeta, EntityMetadataMap, EntityMetadataValue};
pub use experience_orb::ExperienceOrb;
pub use health::Health;
pub use instance_ref::InstanceRef;
pub use inventory::{PlayerInventory, QuickCraftState, convert_minestom_slot_to_window_slot};
pub use item_entity::ItemEntity;
pub use living::{EntityAIGroup, HurtResult, Living};
pub use player::{GameMode, Player};
pub use position::Position;
pub use projectile::EntityProjectile;
pub use velocity::Velocity;
