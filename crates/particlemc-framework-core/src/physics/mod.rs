// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 物理模块：碰撞盒、形状抽象与射线检测的几何基础。
//!
//! 提供轴对齐包围盒 [`Aabb`]（相交 / 方块重叠 / 平移）、实体碰撞盒构造
//! [`entity_box`]、方块形状抽象 [`Shape`]、方块形状查询 [`block_shape`] 与
//! DDA 射线检测 [`Ray`] / [`raycast`]。逐轴位移与碰撞求解
//! （[`crate::system::physics::move_axis`] /
//! [`crate::system::physics::move_and_collide`]）位于 `system::physics`，
//! 此处仅承载纯几何类型。
//!
//! 变更标识符：`complete-partial-framework-capabilities`（T4 碰撞增强）。
//! 见 `.specs/complete-partial-framework-capabilities/spec.md`。
//! 后续扩展（扫掠检测/空气动力学/物理结果）见 `.specs/complete-collision-physics/spec.md`。

pub mod aabb;
pub mod aerodynamics;
pub mod block_collision;
pub mod block_shapes;
pub mod entity_collision;
pub mod ray;
pub mod result;
pub mod shape;
pub mod sweep_result;
pub mod utils;

pub use aabb::{
    Aabb, DEFAULT_ENTITY_HEIGHT, DEFAULT_ENTITY_WIDTH, box_from_foot_center, entity_box,
};
pub use aerodynamics::Aerodynamics;
pub use block_collision::block_shape;
pub use block_shapes::{Box6, box6_to_aabb, shape_boxes};
pub use entity_collision::{EntityCollisionResult, check_entity_collision, max_entity_diagonal};
pub use ray::{Ray, raycast};
pub use result::PhysicsResult;
pub use shape::Shape;
pub use sweep_result::SweepResult;
pub use utils::{
    BlockGetter, NearbyEntitiesFn, WorldBorderFn, can_place_block_at, simulate_movement,
};
