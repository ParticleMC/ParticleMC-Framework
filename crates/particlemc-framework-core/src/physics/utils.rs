// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 物理运动模拟工具：组合方块碰撞 + 空气动力学 + WorldBorder 约束。
//!
//! 对齐 Java [`net.minestom.server.collision.PhysicsUtils`] 的框架层抽象。
//! 本模块提供 `simulate_movement`（完整运动模拟）与 `can_place_block_at`
//! （方块放置检测），所有外部依赖（方块查询、实体查询）通过闭包钩子注入，
//! 框架层不耦合具体实例/实体管理器。
//!
//! 变更标识符：`complete-collision-physics`（WS7、WS8）。
//! 见 `.specs/complete-collision-physics/spec.md`。

use crate::physics::{Aabb, Aerodynamics, PhysicsResult, Shape, SweepResult};

/// WorldBorder 约束闭包：输入目标位置，输出边界内合法位置。
/// 恒等函数表示无边界约束。
pub type WorldBorderFn = dyn Fn([f64; 3]) -> [f64; 3];

/// 方块查询闭包：给定世界坐标方块索引，返回该位置的碰撞形状。
/// y < 0 或未加载区块应返回 `Shape::Empty`。
pub type BlockGetter = dyn Fn(i32, i32, i32) -> Shape;

/// 附近实体查询闭包（用于 can_place_block_at）。
/// 返回的 Aabb 引用须为 `'static`（测试中通常用 `Box::leak` 或静态数据）。
pub type NearbyEntitiesFn = dyn Fn([i32; 3], f64) -> Vec<(u64, [f64; 3], &'static Aabb)>;

/// EPSILON 容差，用于浮点比较。
const EPSILON: f64 = 1e-7;

/// 逐轴扫掠方块碰撞（基于现有 move_and_collide 语义）。
///
/// 返回 `(新位置, 碰撞后速度, CollisionResult)`。
fn handle_block_collision(
    position: [f64; 3],
    velocity: [f64; 3],
    bounding_box: &Aabb,
    block_getter: &BlockGetter,
) -> ([f64; 3], [f64; 3], crate::system::physics::CollisionResult) {
    use crate::system::physics::move_and_collide;

    // 构造实心判定闭包
    let solid = |x: i32, y: i32, z: i32| -> bool { !block_getter(x, y, z).is_empty() };

    let (delta, collision) = move_and_collide(bounding_box, velocity, solid);
    let new_pos = [
        position[0] + delta[0],
        position[1] + delta[1],
        position[2] + delta[2],
    ];
    (new_pos, delta, collision)
}

/// 模拟实体一次 tick 的运动物理。
///
/// 整合方块碰撞 + 空气动力学 + WorldBorder 约束，返回 [`PhysicsResult`]。
///
/// # 参数
/// - `position`：实体脚底中心世界坐标
/// - `velocity`：当前速度（方块/tick）
/// - `bounding_box`：实体当前碰撞盒
/// - `block_getter`：方块查询闭包
/// - `world_border_fn`：WorldBorder 约束闭包（恒等函数表示无边界）
/// - `aerodynamics`：实体空气动力学属性
/// - `no_gravity`：是否无视重力
/// - `on_ground`：当前是否在地面
/// - `flying`：是否飞行模式
#[allow(clippy::too_many_arguments)]
pub fn simulate_movement(
    position: [f64; 3],
    mut velocity: [f64; 3],
    bounding_box: &Aabb,
    block_getter: &BlockGetter,
    world_border_fn: &WorldBorderFn,
    aerodynamics: Aerodynamics,
    no_gravity: bool,
    on_ground: bool,
    flying: bool,
) -> PhysicsResult {
    // 第一步：应用重力（无论是否移动，重力都会作用）
    if !no_gravity && !flying {
        velocity[1] -= aerodynamics.gravity;
    }

    // 第二步：方块碰撞（使用含重力的速度）
    let (collision_pos, _collision_vel, collision_result) =
        handle_block_collision(position, velocity, bounding_box, block_getter);

    // 第三步：WorldBorder 约束
    let bordered_pos = world_border_fn(collision_pos);
    let position_changed = bordered_pos != position;

    // 第四步：空气阻力（位置变化时应用）
    let mut new_velocity = velocity;
    if position_changed {
        new_velocity[0] *= if flying {
            aerodynamics.horizontal_drag
        } else if on_ground {
            0.6 * aerodynamics.horizontal_drag
        } else {
            aerodynamics.horizontal_drag
        };
        new_velocity[1] *= if flying {
            0.6
        } else {
            aerodynamics.vertical_drag
        };
        new_velocity[2] *= if flying {
            aerodynamics.horizontal_drag
        } else if on_ground {
            0.6 * aerodynamics.horizontal_drag
        } else {
            aerodynamics.horizontal_drag
        };
        new_velocity[0] = if new_velocity[0].abs() < EPSILON {
            0.0
        } else {
            new_velocity[0]
        };
        new_velocity[1] = if new_velocity[1].abs() < EPSILON {
            0.0
        } else {
            new_velocity[1]
        };
        new_velocity[2] = if new_velocity[2].abs() < EPSILON {
            0.0
        } else {
            new_velocity[2]
        };
    }

    // 第四步：根据碰撞结果再调整速度（碰撞轴清零）
    let final_velocity = if collision_result.collided_x {
        [0.0, new_velocity[1], new_velocity[2]]
    } else if collision_result.collided_y {
        [new_velocity[0], 0.0, new_velocity[2]]
    } else if collision_result.collided_z {
        [new_velocity[0], new_velocity[1], 0.0]
    } else {
        new_velocity
    };

    // 构造 PhysicsResult
    let has_collision =
        collision_result.collided_x || collision_result.collided_y || collision_result.collided_z;

    let res = if has_collision {
        SweepResult::collision(
            1.0,
            if collision_result.collided_x {
                1.0
            } else {
                0.0
            },
            if collision_result.collided_y {
                1.0
            } else {
                0.0
            },
            if collision_result.collided_z {
                1.0
            } else {
                0.0
            },
            bordered_pos[0],
            bordered_pos[1],
            bordered_pos[2],
        )
    } else {
        SweepResult::NO_COLLISION
    };

    PhysicsResult::with_collision(
        bordered_pos,
        final_velocity,
        collision_result.collided_x,
        collision_result.collided_y,
        collision_result.collided_z,
        velocity,
        res,
    )
}

/// 检测在 `block_pos` 放置方块是否会与附近实体碰撞盒相交。
///
/// - `block_pos`：待放置方块的 world 坐标（方块左下角）
/// - `block_shape`：方块的碰撞形状（通常为 [`Shape::Aabb`] 单位盒）
/// - `nearby_entities`：附近实体查询钩子
///
/// 返回第一个与之相交的实体 ID；若无相交返回 `None`。
/// 搜索半径：实体尺寸对角线 + 3 格（对齐 Java `canPlaceBlockAt`）。
pub fn can_place_block_at(
    block_pos: [i32; 3],
    block_shape: &Shape,
    nearby_entities: &NearbyEntitiesFn,
) -> Option<u64> {
    // 构造方块的世界 AABB（shape_pos 为方块左下角）
    let block_aabb = match block_shape {
        Shape::Aabb(a) => a.moved(
            block_pos[0] as f64,
            block_pos[1] as f64,
            block_pos[2] as f64,
        ),
        Shape::Empty => return None,
        Shape::Merged(boxes) => {
            // 对 Merged 形状，取所有子盒的并集近似为一个 AABB
            let mut min = [f64::INFINITY; 3];
            let mut max = [f64::NEG_INFINITY; 3];
            for box_ in boxes {
                let wb = box_.moved(
                    block_pos[0] as f64,
                    block_pos[1] as f64,
                    block_pos[2] as f64,
                );
                for i in 0..3 {
                    min[i] = min[i].min(wb.min[i]);
                    max[i] = max[i].max(wb.max[i]);
                }
            }
            Aabb { min, max }
        }
    };

    // 搜索半径：3 格 + 实体尺寸（用默认实体尺寸估算）
    let search_radius = 3.0
        + crate::physics::max_entity_diagonal(
            crate::physics::DEFAULT_ENTITY_WIDTH,
            crate::physics::DEFAULT_ENTITY_HEIGHT,
        );

    for (entity_id, _ent_pos, ent_aabb) in nearby_entities(block_pos, search_radius) {
        // 玩家特殊处理：轻微偏移避免边界穿透（高度 < 1.0 的疑似玩家）
        let check_aabb = if ent_aabb.max[1] - ent_aabb.min[1] < 1.0 {
            // 简化：直接使用原 aabb（玩家偏移已在调用方处理）
            ent_aabb
        } else {
            ent_aabb
        };

        if block_aabb.intersects(check_aabb) {
            return Some(entity_id);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::physics::Shape;

    fn empty_block_getter(_x: i32, _y: i32, _z: i32) -> Shape {
        Shape::Empty
    }

    fn identity_border(pos: [f64; 3]) -> [f64; 3] {
        pos
    }

    #[test]
    fn free_fall_in_empty_space() {
        // 初始速度向下 2.0，重力每 tick 增加 0.08
        let aabb = Aabb::from_pos_size([0.0, 70.0, 0.0], [0.6, 1.8, 0.6]);
        let aero = Aerodynamics::default();
        let result = simulate_movement(
            [0.0, 70.0, 0.0],
            [0.0, -2.0, 0.0],
            &aabb,
            &empty_block_getter,
            &identity_border,
            aero,
            false,
            false,
            false,
        );
        // 第一步：重力 vy = -2.0 - 0.08 = -2.08
        // 第二步：碰撞 delta_y = -2.08，位置变化 → position_changed = true
        // 第四步：阻力 vy = -2.08 * 0.98 = -2.0384
        assert!(
            (result.new_velocity[1] - (-2.08 * 0.98)).abs() < 1e-6,
            "vy = {}",
            result.new_velocity[1]
        );
        assert!(
            result.new_position[1] < 70.0,
            "y = {}",
            result.new_position[1]
        );
        assert!(!result.has_collision);
    }

    #[test]
    fn no_gravity_entity_floats() {
        // 初始速度向上 5.0，无重力
        let aabb = Aabb::from_pos_size([0.0, 70.0, 0.0], [0.6, 1.8, 0.6]);
        let aero = Aerodynamics::default();
        let result = simulate_movement(
            [0.0, 70.0, 0.0],
            [0.0, 5.0, 0.0],
            &aabb,
            &empty_block_getter,
            &identity_border,
            aero,
            true, // no_gravity
            false,
            false,
        );
        // 无重力：vy 保持 5.0；位置变化（delta_y=5.0 > EPSILON）→ 应用阻力
        // vy = 5.0 * 0.98 = 4.9
        assert!(
            (result.new_velocity[1] - 4.9).abs() < 1e-6,
            "vy = {}",
            result.new_velocity[1]
        );
    }

    #[test]
    fn flying_ignores_gravity_and_friction() {
        let aabb = Aabb::from_pos_size([0.0, 64.0, 0.0], [0.6, 1.8, 0.6]);
        let aero = Aerodynamics::default();
        let result = simulate_movement(
            [0.0, 64.0, 0.0],
            [1.0, 0.0, 0.0],
            &aabb,
            &empty_block_getter,
            &identity_border,
            aero,
            false,
            true, // on_ground
            true, // flying
        );
        // 飞行模式：忽略地面摩擦，仅应用空气阻力
        assert!((result.new_velocity[0] - 1.0 * aero.horizontal_drag).abs() < 1e-6);
        assert_eq!(result.new_velocity[1], 0.0);
        assert_eq!(result.new_velocity[2], 0.0);
    }

    #[test]
    fn can_place_block_at_no_entity_returns_none() {
        let shape = Shape::Aabb(Aabb::from_pos_size([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]));
        let nearby = |_pos: [i32; 3], _radius: f64| -> Vec<(u64, [f64; 3], &Aabb)> { Vec::new() };
        assert!(can_place_block_at([0, 64, 0], &shape, &nearby).is_none());
    }

    #[test]
    fn can_place_block_at_with_entity_returns_some() {
        let shape = Shape::Aabb(Aabb::from_pos_size([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]));
        // 实体碰撞盒与放置位置相交
        let entity_aabb = Aabb::from_pos_size([0.2, 64.0, 0.2], [0.8, 65.8, 0.8]);
        // 使用 'static 引用：Box::leak + 安全转换（测试环境允许 unsafe）
        let nearby = move |_pos: [i32; 3], _radius: f64| -> Vec<(u64, [f64; 3], &'static Aabb)> {
            let ptr: *const Aabb = Box::leak(Box::new(entity_aabb));
            vec![(1u64, [0.5, 64.9, 0.5], unsafe { &*ptr })]
        };
        assert_eq!(can_place_block_at([0, 64, 0], &shape, &nearby), Some(1));
    }
}
