// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! tick 管线第五步：逐轴碰撞物理（重力 + 地面 / 墙面碰撞）。
//!
//! 对每个持有 `Position` / `Velocity` / `InstanceRef` 的实体，先求其碰撞盒
//! （[`crate::physics::entity_box`]），再沿 x → z → y 三轴逐轴平移并做方块
//! 碰撞：位移与实心方块重叠时贴壁（位置停在碰撞面）并清零该轴速度。
//! [`integrate_y`] 保留竖直积分语义（落地贴地），[`move_axis`] 为纯函数，
//! 便于单测；[`move_and_collide`] 在其上包装三轴整体求解。实心判定统一走
//! [`crate::physics::block_shape`]（内部落到 `BlockRegistry::is_solid`），
//! y < 0 或未加载区块视为非实心（空气）。
//!
//! 变更标识符：`complete-partial-framework-capabilities`（T4 碰撞增强）。
//! 见 `.specs/complete-partial-framework-capabilities/spec.md`。

use crate::prelude::{Query, Res, ResMut, Shared};

use crate::component::{EntityMeta, InstanceRef, Position, Velocity};
use crate::instance::ChunkStore;
use crate::physics::{
    Aabb, DEFAULT_ENTITY_HEIGHT, DEFAULT_ENTITY_WIDTH, block_shape, box_from_foot_center,
    entity_box,
};
use crate::resource::registries::{BlockRegistry, EntityTypeRegistry};

/// 每 tick 的重力加速度（方块 / tick²，简化模型）。
pub const GRAVITY: f64 = 0.08;

/// 脚下实心探测位移：向下微探极小距离，若发生碰撞即视为「脚下有地面」。
const GROUND_PROBE: f64 = 1e-6;

/// 单轴扫描的方块跨度上限（含边界，防御性限制候选方块数）。
const SPAN_LIMIT: i32 = 3;

/// 单轴竖直积分：给定当前 y、竖直速度、脚下是否实心，返回 (新 y, 新速度)。
///
/// - 脚下空心：持续加速下落（速度递减、y 递减）。
/// - 脚下实心且仍下坠：贴附地面（y 保持、速度归零）。
#[must_use]
pub fn integrate_y(pos_y: f64, vel_y: f64, solid_below: bool) -> (f64, f64) {
    let new_vel = vel_y - GRAVITY;
    let new_y = pos_y + new_vel;
    if solid_below && new_y <= pos_y {
        // 落地：贴附当前高度，竖直速度归零
        (pos_y, 0.0)
    } else {
        (new_y, new_vel)
    }
}

/// 沿单轴平移碰撞盒并做方块碰撞检测。
///
/// 返回 `(修正后的位移, 是否碰撞)`：
/// - 无碰撞：返回 `(amount, false)`，位移原样生效。
/// - 碰撞：位移修正到碰撞面（贴壁 / 贴地），返回 `(修正后的位移, true)`。
///
/// 方块探测只检查平移前后 AABB 覆盖的方块范围（每轴跨度受 [`SPAN_LIMIT`]
/// 限制，整体至多 27 个候选），不扫全区块。`solid(x, y, z)` 返回指定
/// 世界坐标方块是否实心。位移为 0 时直接返回 `(0.0, false)`。
#[must_use]
pub fn move_axis(
    axis: usize,
    amount: f64,
    aabb: Aabb,
    solid: &dyn Fn(i32, i32, i32) -> bool,
) -> (f64, bool) {
    if amount == 0.0 {
        return (0.0, false);
    }
    let target = match axis {
        0 => aabb.moved(amount, 0.0, 0.0),
        1 => aabb.moved(0.0, amount, 0.0),
        _ => aabb.moved(0.0, 0.0, amount),
    };
    // 平移前后覆盖的方块范围（min 向下取整、max 向上取整），跨度受限防溢出。
    let x0 = aabb.min[0].min(target.min[0]).floor() as i32;
    let y0 = aabb.min[1].min(target.min[1]).floor() as i32;
    let z0 = aabb.min[2].min(target.min[2]).floor() as i32;
    let x1 = (aabb.max[0].max(target.max[0]).ceil() as i32).min(x0.saturating_add(SPAN_LIMIT));
    let y1 = (aabb.max[1].max(target.max[1]).ceil() as i32).min(y0.saturating_add(SPAN_LIMIT));
    let z1 = (aabb.max[2].max(target.max[2]).ceil() as i32).min(z0.saturating_add(SPAN_LIMIT));

    let mut corrected = amount;
    let mut hit = false;
    for bx in x0..x1 {
        for by in y0..y1 {
            for bz in z0..z1 {
                if !solid(bx, by, bz) || !target.overlaps_block(bx, by, bz) {
                    continue;
                }
                hit = true;
                corrected = match axis {
                    0 => {
                        if amount > 0.0 {
                            corrected.min(bx as f64 - aabb.max[0])
                        } else {
                            corrected.max((bx + 1) as f64 - aabb.min[0])
                        }
                    }
                    1 => {
                        if amount > 0.0 {
                            corrected.min(by as f64 - aabb.max[1])
                        } else {
                            corrected.max((by + 1) as f64 - aabb.min[1])
                        }
                    }
                    _ => {
                        if amount > 0.0 {
                            corrected.min(bz as f64 - aabb.max[2])
                        } else {
                            corrected.max((bz + 1) as f64 - aabb.min[2])
                        }
                    }
                };
            }
        }
    }
    if !hit {
        return (amount, false);
    }
    // 修正位移钳制在 [0, amount] 或 [amount, 0]，防御已重叠的退化状态。
    let corrected = if amount > 0.0 {
        corrected.clamp(0.0, amount)
    } else {
        corrected.clamp(amount, 0.0)
    };
    (corrected, true)
}

/// 三轴碰撞结果：各轴是否发生碰撞。
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct CollisionResult {
    /// x 轴（水平）发生碰撞。
    pub collided_x: bool,
    /// y 轴（竖直）发生碰撞。
    pub collided_y: bool,
    /// z 轴（水平）发生碰撞。
    pub collided_z: bool,
}

/// 三轴整体移动求解：在 [`move_axis`] 逐轴分解基础上返回实际位移与碰撞结果。
///
/// 按 x → z → y 顺序逐轴求解（与 [`physics`] 系统一致），每轴用前一轴的
/// 修正结果更新碰撞盒后再求下一轴，天然实现「斜向贴墙时沿墙滑动」的
/// 阶式位移语义。`is_solid(x, y, z)` 判定方块是否实心，调用方应经由
/// [`block_shape`] 查询（替换裸「`!= 0` 即实心」的旧判定）。
///
/// 返回值：`(实际位移 [dx, dy, dz], 三轴碰撞结果)`。实际位移各分量等于
/// 输入 `delta` 的对应分量或在碰撞面处被钳制，不再叠加速度清零等副作用。
#[must_use]
pub fn move_and_collide(
    aabb: &Aabb,
    delta: [f64; 3],
    is_solid: impl Fn(i32, i32, i32) -> bool,
) -> ([f64; 3], CollisionResult) {
    // x 轴。
    let (dx, collided_x) = move_axis(0, delta[0], *aabb, &is_solid);
    let mut current = aabb.moved(dx, 0.0, 0.0);
    // z 轴（携带 x 轴修正结果）。
    let (dz, collided_z) = move_axis(2, delta[2], current, &is_solid);
    current = current.moved(0.0, 0.0, dz);
    // y 轴（携带 x / z 轴修正结果）。
    let (dy, collided_y) = move_axis(1, delta[1], current, &is_solid);

    (
        [dx, dy, dz],
        CollisionResult {
            collided_x,
            collided_y,
            collided_z,
        },
    )
}

/// 对每个受物理影响的实体施加重力与逐轴方块碰撞。
///
/// 本系统运行于实例 World 内（R11.2 实体迁入后），直接读取本 World 的
/// [`ChunkStore`] 与经 `Shared<T>` 注入的只读注册表；区块数据随实例 World
/// 存放，无需跨 World 回主 World 取。
///
/// 碰撞盒尺寸：带 `EntityMeta`（含实体类型）的实体按注册表尺寸，否则按
/// 玩家缺省 0.6 × 1.8。x / z 轴由速度直接驱动位移，y 轴先经
/// [`integrate_y`] 积分再碰撞；任一轴碰撞贴壁即清零该轴速度。
pub fn physics(
    mut entities: Query<(
        &mut Position,
        &mut Velocity,
        &InstanceRef,
        Option<&EntityMeta>,
    )>,
    mut store: ResMut<ChunkStore>,
    block_registry: Res<Shared<BlockRegistry>>,
    entity_types: Res<Shared<EntityTypeRegistry>>,
    q_mark: Query<(&Velocity, &Position)>,
) {
    // 第一遍（标记动态分片）：遍历本实例 World 全部实体，将「本帧可能移动」的
    // 实体所在区块记入 ChunkStore 动态分片，第二遍据此「仅遍历动态分片」积分；
    // 静止且脚下有支撑的实体本帧不会移动，跳过其积分（位置不变，安全）。
    // 「可能移动」= 速度任一分量为非零（已在动），或静止但脚下无支撑（悬空，
    // 下一刻将受重力下落）。二者皆非者（静止且脚踏实地）方才跳过——否则悬空
    // 静止实体将永不坠落（安全红线）。
    store.clear_dynamic();
    let registry = &*block_registry;
    for (vel, pos) in q_mark.iter() {
        let moving = vel.x != 0.0 || vel.y != 0.0 || vel.z != 0.0;
        // 静止实体：探针脚下一格是否实心（提供地面支撑）。无支撑即悬空，本帧
        // 将受重力下落，须标记动态分片参与积分。
        let supported = if !moving {
            let bx = pos.x.div_euclid(1.0) as i32;
            let by = (pos.y - 1.0).floor() as i32;
            let bz = pos.z.div_euclid(1.0) as i32;
            let id = store.get_block_id_world(bx, by, bz);
            !block_shape(id, registry).is_empty()
        } else {
            false
        };
        if moving || !supported {
            let cx = pos.x.div_euclid(16.0) as i32;
            let cz = pos.z.div_euclid(16.0) as i32;
            store.mark_chunk_dynamic(cx, cz);
        }
    }

    // 第二遍（物理积分）：遍历全部实体；仅当实体所在区块在动态分片时执行物理，
    // 否则该区块本帧无移动实体，直接跳过（静止实体积分后位置不变，故安全）。
    for (pos, vel, _inst, meta) in entities.iter_mut() {
        let cx = pos.x.div_euclid(16.0) as i32;
        let cz = pos.z.div_euclid(16.0) as i32;
        if !store.is_chunk_dynamic(cx, cz) {
            continue;
        }
        // 实心判定闭包：经 `block_shape` 查询（y < 0 或区块未加载时
        // `get_block_id_world` 返回 0 → 空气 → Empty），而非裸「id != 0」判定。
        let solid = |x: i32, y: i32, z: i32| {
            let id = store.get_block_id_world(x, y, z);
            !block_shape(id, &block_registry).is_empty()
        };
        // 碰撞盒：有实体类型元数据按注册表尺寸，否则玩家缺省 0.6×1.8。
        let aabb = match meta.and_then(|m| m.entity_type) {
            Some(ty) => entity_box(&ty, &entity_types, pos),
            None => box_from_foot_center(
                [pos.x, pos.y, pos.z],
                DEFAULT_ENTITY_WIDTH,
                DEFAULT_ENTITY_HEIGHT,
            ),
        };

        // x 轴：位移 + 碰撞贴壁清速。
        let (dx, hit_x) = move_axis(0, vel.x, aabb, &solid);
        let mut aabb = aabb.moved(dx, 0.0, 0.0);
        if hit_x {
            vel.x = 0.0;
        }

        // z 轴：同理。
        let (dz, hit_z) = move_axis(2, vel.z, aabb, &solid);
        aabb = aabb.moved(0.0, 0.0, dz);
        if hit_z {
            vel.z = 0.0;
        }

        // y 轴：先经 integrate_y 积分竖直运动，再做碰撞检测；落地（任一方向
        // 撞到方块）清零 vel.y。
        let foot_solid = move_axis(1, -GROUND_PROBE, aabb, &solid).1;
        let (new_y, new_vel_y) = integrate_y(pos.y, vel.y, foot_solid);
        let dy = new_y - pos.y;
        let (ay, hit_y) = move_axis(1, dy, aabb, &solid);
        aabb = aabb.moved(0.0, ay, 0.0);
        if hit_y {
            vel.y = 0.0;
        } else {
            vel.y = new_vel_y;
        }

        // 由最终碰撞盒反推脚底中心坐标。
        pos.x = (aabb.min[0] + aabb.max[0]) / 2.0;
        pos.y = aabb.min[1];
        pos.z = (aabb.min[2] + aabb.max[2]) / 2.0;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    use std::sync::Arc;

    use crate::app::App;
    use crate::prelude::Entity;

    use crate::instance::Chunk;
    use crate::instance::chunk_store::ChunkStore;
    use crate::resource::EntityType;
    use crate::resource::registries::{
        BlockDefinition, EntityTypeDefinition, EntityTypeRegistry, Registry,
    };
    use particlemc_framework_ecs::scheduler::WorldId;

    /// 构造最小方块注册表：air=0、stone=1。
    fn test_block_registry() -> BlockRegistry {
        let toml = r#"
            [[entry]]
            id = 0
            name = "minecraft:air"

            [[entry]]
            id = 1
            name = "minecraft:stone"
        "#;
        let inner = Registry::<BlockDefinition>::from_toml_str(toml).unwrap();
        BlockRegistry(inner)
    }

    /// 实心判定闭包：按预置的实心方块集合查询。
    fn solid_at(blocks: &[(i32, i32, i32)], x: i32, y: i32, z: i32) -> bool {
        blocks
            .iter()
            .any(|&(bx, by, bz)| bx == x && by == y && bz == z)
    }

    // ---------- integrate_y 既有测试（语义不变） ----------

    #[test]
    fn free_fall_accelerates_downward() {
        // 空中（脚下空心）：速度持续减小、y 持续下降
        let (y1, v1) = integrate_y(100.0, 0.0, false);
        assert!(v1 < 0.0);
        assert!(y1 < 100.0);
        let (y2, v2) = integrate_y(y1, v1, false);
        assert!(v2 < v1);
        assert!(y2 < y1);
    }

    #[test]
    fn lands_on_ground_and_stops() {
        // 模拟从空中下落直到触地，断言最终不再下降
        let mut y = 70.0;
        let mut v = 0.0;
        // 构造一个脚下实心的情形：当 y 逼近 64 时视为落地基准
        for _ in 0..2000 {
            // 模拟地面：y <= 64 时脚下实心
            let solid = y <= 64.0;
            let (new_y, new_v) = integrate_y(y, v, solid);
            y = new_y;
            v = new_v;
            if v == 0.0 && solid {
                break;
            }
        }
        assert_eq!(v, 0.0);
        // 落地后 y 不再下降
        let (after_y, after_v) = integrate_y(y, v, y <= 64.0);
        assert_eq!(after_v, 0.0);
        assert_eq!(after_y, y);
    }

    #[test]
    fn standing_still_on_ground_stays_put() {
        // 已站在地面（脚下实心）且速度为零：保持不动
        let (y, v) = integrate_y(64.0, 0.0, true);
        assert_eq!(y, 64.0);
        assert_eq!(v, 0.0);
    }

    // ---------- move_axis 纯函数测试 ----------

    #[test]
    fn move_axis_zero_amount_returns_zero() {
        let aabb = Aabb::from_pos_size([0.0, 64.0, 0.0], [0.6, 1.8, 0.6]);
        let solid = |_: i32, _: i32, _: i32| false;
        assert_eq!(move_axis(0, 0.0, aabb, &solid), (0.0, false));
        assert_eq!(move_axis(1, 0.0, aabb, &solid), (0.0, false));
    }

    #[test]
    fn move_axis_free_movement_passthrough() {
        let blocks: &[(i32, i32, i32)] = &[];
        let solid = |x: i32, y: i32, z: i32| solid_at(blocks, x, y, z);
        let aabb = Aabb::from_pos_size([4.7, 64.0, 0.0], [0.6, 1.8, 0.6]);
        // 前方无方块：位移原样生效。
        assert_eq!(move_axis(0, 0.5, aabb, &solid), (0.5, false));
        assert_eq!(move_axis(1, -0.3, aabb, &solid), (-0.3, false));
        assert_eq!(move_axis(2, 0.2, aabb, &solid), (0.2, false));
    }

    #[test]
    fn move_axis_blocks_into_wall_positive() {
        // 墙方块占据 x∈[10,11)，实体向右移动应停在碰撞面 x=10。
        let blocks: &[(i32, i32, i32)] = &[(10, 64, 0), (10, 65, 0)];
        let solid = |x: i32, y: i32, z: i32| solid_at(blocks, x, y, z);
        let aabb = Aabb::from_pos_size([9.2, 64.0, 0.0], [0.6, 1.8, 0.6]);
        let (dx, hit) = move_axis(0, 1.5, aabb, &solid);
        assert!(hit);
        // 修正后 max[0] == 10.0（墙面）。
        assert!((aabb.max[0] + dx - 10.0).abs() < 1e-9);
        assert!(dx > 0.0 && dx < 1.5);
    }

    #[test]
    fn move_axis_blocks_into_wall_negative() {
        // 墙方块占据 x∈[10,11)，实体向左移动畅通；从右侧撞向它应停在 x=11。
        let blocks: &[(i32, i32, i32)] = &[(10, 64, 0)];
        let solid = |x: i32, y: i32, z: i32| solid_at(blocks, x, y, z);
        let aabb = Aabb::from_pos_size([11.2, 64.0, 0.0], [0.6, 1.8, 0.6]);
        let (dx, hit) = move_axis(0, -0.8, aabb, &solid);
        assert!(hit);
        // 修正后 min[0] == 11.0（墙另一面）。
        assert!((aabb.min[0] + dx - 11.0).abs() < 1e-9);
        assert!(dx < 0.0 && dx > -0.8);
    }

    #[test]
    fn move_axis_lands_on_ground_top() {
        // 地面方块占据 y∈[64,65)，下坠实体应停在 y=65（地面顶面）。
        let blocks: &[(i32, i32, i32)] = &[(0, 64, 0)];
        let solid = |x: i32, y: i32, z: i32| solid_at(blocks, x, y, z);
        let aabb = Aabb::from_pos_size([0.0, 66.0, 0.0], [0.6, 1.8, 0.6]);
        let (dy, hit) = move_axis(1, -2.0, aabb, &solid);
        assert!(hit);
        // 修正后 min[1] == 65.0（落地贴地）。
        assert!((aabb.min[1] + dy - 65.0).abs() < 1e-9);
        assert!(dy < 0.0 && dy > -2.0);
    }

    #[test]
    fn move_axis_hits_ceiling_positive() {
        // 天花板方块占据 y∈[70,71)，上跳实体应停在 y=70。
        let blocks: &[(i32, i32, i32)] = &[(0, 70, 0)];
        let solid = |x: i32, y: i32, z: i32| solid_at(blocks, x, y, z);
        let aabb = Aabb::from_pos_size([0.0, 68.0, 0.0], [0.6, 1.8, 0.6]);
        let (dy, hit) = move_axis(1, 2.0, aabb, &solid);
        assert!(hit);
        // 修正后 max[1] == 70.0（天花板底面）。
        assert!((aabb.max[1] + dy - 70.0).abs() < 1e-9);
        assert!(dy > 0.0 && dy < 2.0);
    }

    // ---------- move_and_collide 三轴整体求解测试 ----------

    #[test]
    fn move_and_collide_free_motion_passthrough() {
        let blocks: &[(i32, i32, i32)] = &[];
        let solid = |x: i32, y: i32, z: i32| solid_at(blocks, x, y, z);
        let aabb = Aabb::from_pos_size([4.7, 64.0, 0.0], [0.6, 1.8, 0.6]);
        let (delta, result) = move_and_collide(&aabb, [0.5, 0.3, -0.2], solid);
        assert_eq!(delta, [0.5, 0.3, -0.2]);
        assert!(!result.collided_x && !result.collided_y && !result.collided_z);
    }

    #[test]
    fn move_and_collide_wall_blocks_x_and_clears_axis() {
        // 墙方块占据 x∈[10,11)，向右移动应停在碰撞面 x=10 并标记 x 碰撞。
        let blocks: &[(i32, i32, i32)] = &[(10, 64, 0), (10, 65, 0)];
        let solid = |x: i32, y: i32, z: i32| solid_at(blocks, x, y, z);
        let aabb = Aabb::from_pos_size([9.2, 64.0, 0.0], [0.6, 1.8, 0.6]);
        let (delta, result) = move_and_collide(&aabb, [1.5, 0.0, 0.0], solid);
        assert!(result.collided_x);
        assert!(!result.collided_y && !result.collided_z);
        // 修正后 max[0] == 10.0（墙面）。
        assert!((aabb.max[0] + delta[0] - 10.0).abs() < 1e-9);
        assert!(delta[0] > 0.0 && delta[0] < 1.5);
    }

    #[test]
    fn move_and_collide_lands_on_ground() {
        // 地面方块占据 y∈[64,65)，下坠应停在 y=65（落地贴地）。
        let blocks: &[(i32, i32, i32)] = &[(0, 64, 0)];
        let solid = |x: i32, y: i32, z: i32| solid_at(blocks, x, y, z);
        let aabb = Aabb::from_pos_size([0.0, 66.0, 0.0], [0.6, 1.8, 0.6]);
        let (delta, result) = move_and_collide(&aabb, [0.0, -2.0, 0.0], solid);
        assert!(result.collided_y);
        assert!(!result.collided_x && !result.collided_z);
        assert!((aabb.min[1] + delta[1] - 65.0).abs() < 1e-9);
        assert!(delta[1] < 0.0 && delta[1] > -2.0);
    }

    #[test]
    fn move_and_collide_diagonal_slides_along_wall() {
        // 斜向撞墙：x 被墙挡下（贴壁），z 自由移动（沿墙滑动）。
        let blocks: &[(i32, i32, i32)] = &[(10, 64, 0), (10, 65, 0)];
        let solid = |x: i32, y: i32, z: i32| solid_at(blocks, x, y, z);
        let aabb = Aabb::from_pos_size([9.2, 64.0, 0.0], [0.6, 1.8, 0.6]);
        let (delta, result) = move_and_collide(&aabb, [1.5, 0.0, 1.5], solid);
        assert!(result.collided_x);
        assert!(!result.collided_y && !result.collided_z);
        // x 贴壁（< 请求位移），z 原样滑动（== 请求位移）。
        assert!(delta[0] < 1.5);
        assert_eq!(delta[2], 1.5);
        // 修正后最终位置：max[0] 贴到墙面。
        assert!((aabb.max[0] + delta[0] - 10.0).abs() < 1e-9);
    }

    #[test]
    fn move_and_collide_corner_hits_y_and_x() {
        // 角落：x=8/9 地面（y=64）+ x=10 墙面（y∈[65,68)）。实体同时下坠并
        // 右移，x 被墙面挡下、y 落地到地面顶面，两轴碰撞均被标记。
        let blocks: &[(i32, i32, i32)] = &[
            (8, 64, 0),
            (9, 64, 0),
            (10, 65, 0),
            (10, 66, 0),
            (10, 67, 0),
        ];
        let solid = |x: i32, y: i32, z: i32| solid_at(blocks, x, y, z);
        let aabb = Aabb::from_pos_size([8.7, 66.0, 0.0], [0.6, 1.8, 0.6]);
        let (delta, result) = move_and_collide(&aabb, [1.5, -2.0, 0.0], solid);
        assert!(result.collided_x && result.collided_y);
        assert!(!result.collided_z);
        // y 停在 65（地面顶面），x 停在 10（墙面）。
        assert!((aabb.min[1] + delta[1] - 65.0).abs() < 1e-9);
        assert!((aabb.max[0] + delta[0] - 10.0).abs() < 1e-9);
    }

    // ---------- 系统级测试 ----------

    /// 构造带最小共享注册表与 `physics` 系统的 App（R11.5：physics 经
    /// `Shared<T>` 只读访问注册表）。
    fn physics_app() -> App {
        let mut app = App::new();
        let block = test_block_registry();
        let et = EntityTypeRegistry::default();
        app.world_mut().insert_shared(Arc::new(block));
        app.world_mut().insert_shared(Arc::new(et));
        app.add_systems(physics);
        app
    }

    /// 在主 World 注入含地面（y=64 一层）与墙（x=2 两格高）的 `ChunkStore`，
    /// 返回占位 `WorldId`（physics 在本 World 内读取 `ChunkStore`，`InstanceRef`
    /// 取值不参与积分）。
    fn spawn_world(app: &mut App, with_wall: bool) -> WorldId {
        let mut store = ChunkStore::new();
        let chunk = Chunk::new(0, 0, 8);
        store.load_chunk(chunk);
        // 地面：y=64，x/z ∈ [0, 4]。
        for x in 0..=4 {
            for z in 0..=4 {
                store.set_block_id_world(x, 64, z, 1);
            }
        }
        if with_wall {
            // 墙：x=2，y ∈ [64, 66)，z ∈ [0, 4]。
            for y in 64..66 {
                for z in 0..=4 {
                    store.set_block_id_world(2, y, z, 1);
                }
            }
        }
        app.world_mut().insert_resource(store);
        WorldId(0)
    }

    /// 生成受物理影响的实体，返回实体 id。
    fn spawn_body(
        app: &mut App,
        _instance: WorldId,
        x: f64,
        y: f64,
        z: f64,
        vel: Velocity,
    ) -> Entity {
        app.world_mut()
            .spawn_bundle((Position::new(x, y, z), vel, InstanceRef(WorldId(0))))
            .id()
    }

    /// 读取实体组件。
    fn body(app: &mut App, entity: Entity) -> (Position, Velocity) {
        let (pos, vel) = {
            let world = app.world_mut();
            let q = world.query::<(&Position, &Velocity), ()>();
            q.get(entity).map(|(p, v)| (*p, *v)).unwrap()
        };
        (pos, vel)
    }

    #[test]
    fn system_falls_to_ground_and_stops() {
        let mut app = physics_app();
        let instance = spawn_world(&mut app, false);
        let entity = spawn_body(&mut app, instance, 0.0, 80.0, 0.0, Velocity::zero());
        // 跑足够多 tick 完成下落与落地。
        for _ in 0..100 {
            app.update();
        }
        let (pos, vel) = body(&mut app, entity);
        // 地面顶面 y=65，落地后 y 停在 65、竖直速度归零。
        assert!((pos.y - 65.0).abs() < 1e-6, "y = {}", pos.y);
        assert_eq!(vel.y, 0.0);
    }

    #[test]
    fn system_free_fall_accelerates_downward() {
        let mut app = physics_app();
        let instance = spawn_world(&mut app, false);
        // 空中远离地面，避免落地干扰。
        let entity = spawn_body(&mut app, instance, 100.0, 200.0, 0.0, Velocity::zero());
        app.update();
        let (_, vel1) = body(&mut app, entity);
        assert!(vel1.y < 0.0, "vy1 = {}", vel1.y);
        app.update();
        let (pos2, vel2) = body(&mut app, entity);
        assert!(vel2.y < vel1.y, "vy2 = {}", vel2.y);
        assert!(pos2.y < 200.0);
    }

    #[test]
    fn system_horizontal_motion_blocked_by_wall() {
        let mut app = physics_app();
        let instance = spawn_world(&mut app, true);
        // 站在地面上向右移动，墙（x=2）前方 1.2 格处起步。
        let entity = spawn_body(
            &mut app,
            instance,
            0.5,
            65.0,
            0.0,
            Velocity::new(0.5, 0.0, 0.0),
        );
        for _ in 0..30 {
            app.update();
        }
        let (pos, vel) = body(&mut app, entity);
        // 墙内边界 x=2，实体宽 0.6 → 中心最远 1.7，且不允许穿过。
        assert!(pos.x <= 1.700_001, "x = {}", pos.x);
        assert!((pos.x - 1.7).abs() < 1e-3, "x = {}", pos.x);
        assert_eq!(vel.x, 0.0);
        // 仍未穿墙且没有掉出地面。
        assert!((pos.y - 65.0).abs() < 1e-6, "y = {}", pos.y);
    }

    #[test]
    fn system_default_size_fits_one_block_corridor() {
        // 无 EntityMeta 的实体按玩家缺省 0.6 宽构造碰撞盒：能穿过 1 格宽走廊
        // （z∈[1,2)），并最终撞上走廊尽头墙停下。
        let mut app = physics_app();
        let mut store = ChunkStore::new();
        let chunk = Chunk::new(0, 0, 8);
        store.load_chunk(chunk);
        for x in 0..=6 {
            for z in 0..=4 {
                store.set_block_id_world(x, 64, z, 1); // 地面
            }
            // 走廊两侧墙：z=0 与 z=2，两格高。
            for y in 64..66 {
                store.set_block_id_world(x, y, 0, 1);
                store.set_block_id_world(x, y, 2, 1);
            }
        }
        // 走廊尽头墙：x=6。
        for y in 64..66 {
            store.set_block_id_world(6, y, 1, 1);
        }
        app.world_mut().insert_resource(store);
        let instance = WorldId(0);
        // 出生在走廊内部（z=1.5，宽 1 格的走廊 z∈[1,2)），向尽头移动。
        let entity = spawn_body(
            &mut app,
            instance,
            0.5,
            65.0,
            1.5,
            Velocity::new(0.5, 0.0, 0.0),
        );
        for _ in 0..30 {
            app.update();
        }
        let (pos, vel) = body(&mut app, entity);
        // 穿过走廊到达尽头墙 x=6：0.6 宽 → 中心停在 5.7。
        assert!((pos.x - 5.7).abs() < 1e-3, "x = {}", pos.x);
        assert_eq!(vel.x, 0.0);
        assert!((pos.y - 65.0).abs() < 1e-6, "y = {}", pos.y);
    }

    #[test]
    fn system_entity_meta_size_from_registry() {
        // 带 EntityMeta（cow 0.9 宽）的实体按注册表尺寸构造碰撞盒：
        // 撞墙停止位置取决于宽度（中心 = 5.0 - 0.45 = 4.55），验证注册表生效。
        let mut app = App::new();
        let block = test_block_registry();
        let et_toml = r#"
            [[entry]]
            id = 0
            name = "minecraft:player"
            width = 0.6
            height = 1.8

            [[entry]]
            id = 1
            name = "minecraft:cow"
            width = 0.9
            height = 1.4
        "#;
        let et_inner = Registry::<EntityTypeDefinition>::from_toml_str(et_toml).unwrap();
        let et = EntityTypeRegistry(et_inner);
        app.world_mut().insert_shared(Arc::new(block));
        app.world_mut().insert_shared(Arc::new(et));
        app.add_systems(physics);

        let mut store = ChunkStore::new();
        let chunk = Chunk::new(0, 0, 8);
        store.load_chunk(chunk);
        for x in 0..=5 {
            for z in 0..=4 {
                store.set_block_id_world(x, 64, z, 1); // 地面
            }
        }
        // 尽头墙：x=5，两格高。
        for y in 64..66 {
            for z in 0..=4 {
                store.set_block_id_world(5, y, z, 1);
            }
        }
        app.world_mut().insert_resource(store);
        let instance = WorldId(0);
        let entity = app
            .world_mut()
            .spawn_bundle((
                Position::new(0.5, 65.0, 0.0),
                Velocity::new(0.5, 0.0, 0.0),
                InstanceRef(instance),
                EntityMeta::new(EntityType::by_id(1)),
            ))
            .id();
        for _ in 0..30 {
            app.update();
        }
        let (pos, vel) = body(&mut app, entity);
        // 0.9 宽撞墙：中心停在 4.55；缺省 0.6 宽则应为 4.7。
        assert!((pos.x - 4.55).abs() < 1e-3, "x = {}", pos.x);
        assert_eq!(vel.x, 0.0);
        assert!((pos.y - 65.0).abs() < 1e-6, "y = {}", pos.y);
    }
}
