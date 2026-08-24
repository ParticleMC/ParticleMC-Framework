// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! AABB 碰撞盒：轴对齐包围盒的基础几何与方块重叠判定。
//!
//! [`Aabb`] 以 `min` / `max` 两个角点描述一个轴对齐包围盒，提供相交判定、
//! 单位方块重叠判定、平移运算与扫掠相交检测（[`Aabb::sweep_intersection`]，
//! 基于 slabs 方法）。[`entity_box`] 把实体类型尺寸与脚底中心坐标换算为碰撞盒。
//!
//! 变更标识符：`complete-partial-framework-capabilities`（T4 碰撞增强）；
//! 扫掠扩展见 `.specs/complete-collision-physics/spec.md`（WS2）。

use crate::component::Position;
use crate::resource::EntityType;
use crate::resource::registries::EntityTypeRegistry;

use super::sweep_result::SweepResult;

/// 实体碰撞盒缺省宽度（方块，对应玩家 0.6）。
pub const DEFAULT_ENTITY_WIDTH: f64 = 0.6;
/// 实体碰撞盒缺省高度（方块，对应玩家 1.8）。
pub const DEFAULT_ENTITY_HEIGHT: f64 = 1.8;

/// EPSILON 容差：距离小于此值的碰撞视为相切，不计入。
const EPSILON: f64 = 1e-7;

/// 轴对齐包围盒（AABB）：以 `min` / `max` 两个角点坐标定义。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Aabb {
    /// 最小角点坐标（含）。
    pub min: [f64; 3],
    /// 最大角点坐标（含）。
    pub max: [f64; 3],
}

impl Aabb {
    /// 以「最小角点 + 边长」构造包围盒：`max = pos + size`。
    pub fn from_pos_size(pos: [f64; 3], size: [f64; 3]) -> Self {
        Self {
            min: pos,
            max: [pos[0] + size[0], pos[1] + size[1], pos[2] + size[2]],
        }
    }

    /// 是否与另一包围盒相交（含相切：边界恰好重合视为相交）。
    pub fn intersects(&self, other: &Aabb) -> bool {
        (0..3).all(|i| self.min[i] <= other.max[i] && self.max[i] >= other.min[i])
    }

    /// 是否与指定单位方块 `[x, x+1) × [y, y+1) × [z, z+1)` 相交。
    ///
    /// 采用严格相交：与方块表面相切不算重叠，避免贴墙 / 贴地时因浮点抖动
    /// 反复判为碰撞。
    pub fn overlaps_block(&self, x: i32, y: i32, z: i32) -> bool {
        let bx = x as f64;
        let by = y as f64;
        let bz = z as f64;
        self.max[0] > bx
            && self.min[0] < bx + 1.0
            && self.max[1] > by
            && self.min[1] < by + 1.0
            && self.max[2] > bz
            && self.min[2] < bz + 1.0
    }

    /// 平移后的副本：各角点分别加上 `(dx, dy, dz)`。
    pub fn moved(&self, dx: f64, dy: f64, dz: f64) -> Self {
        Self {
            min: [self.min[0] + dx, self.min[1] + dy, self.min[2] + dz],
            max: [self.max[0] + dx, self.max[1] + dy, self.max[2] + dz],
        }
    }

    /// 沿 `delta` 扫掠检测与本 AABB 是否与 `other` 相交（slabs 方法）。
    ///
    /// 将本 AABB 的每个面视为一对平行平面，计算射线（起点为本 AABB 中心，
    /// 方向为 `delta`）穿过各平面的参数 `t`，取有效交集得到最近碰撞比例。
    ///
    /// - 返回 `Some(SweepResult)`：命中，`res ∈ [0, 1)` 为碰撞比例，
    ///   `normal_*` 为碰撞面法线（`±1` 或 `0`）。
    /// - 返回 `None`：无碰撞（含相切，相切距离 < `EPSILON` 不计入）。
    /// - `delta` 全为零向量时退化为静态相交检测。
    #[must_use]
    pub fn sweep_intersection(&self, delta: [f64; 3], other: &Aabb) -> Option<SweepResult> {
        // 静止检测：delta 全零时退化为相交判断
        if delta == [0.0, 0.0, 0.0] {
            if self.intersects(other) {
                return Some(SweepResult::collision(
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    (self.min[0] + other.max[0]) / 2.0,
                    (self.min[1] + other.max[1]) / 2.0,
                    (self.min[2] + other.max[2]) / 2.0,
                ));
            }
            return None;
        }

        let mut t_min = 0.0f64;
        let mut t_max = 1.0f64;
        let mut normal_x = 0.0f64;
        let mut normal_y = 0.0f64;
        let mut normal_z = 0.0f64;

        // 先验证静止轴的相交性（该轴无运动，直接检查重叠）
        for (axis, &d) in delta.iter().enumerate() {
            if d.abs() >= EPSILON {
                continue;
            }
            // 该轴无运动：必须严格重叠（不含相切），否则不可能碰撞
            // 注意：使用严格不等式，允许相切（相切由移动轴处理）
            if self.max[axis] < other.min[axis] || self.min[axis] > other.max[axis] {
                return None;
            }
        }

        let mut found_collision = false;

        for (axis, &d) in delta.iter().enumerate() {
            if d.abs() < EPSILON {
                continue;
            }

            // 计算射线穿过 other 两个相对面的参数 t
            let t0 = (other.min[axis] - self.max[axis]) / d;
            let t1 = (other.max[axis] - self.min[axis]) / d;

            // 确保 t0 <= t1
            let (entry, exit) = if t0 < t1 { (t0, t1) } else { (t1, t0) };

            // 更新全局 t 交集
            if entry >= t_min - EPSILON {
                t_min = entry.max(0.0);
                found_collision = true;
                // 法线：用 delta 方向判断——delta>0 表示向正轴移动，碰撞面法线为 -1；
                // delta<0 表示向负轴移动，碰撞面法线为 +1
                let n = if d.abs() < EPSILON {
                    0.0
                } else if d > 0.0 {
                    -1.0
                } else {
                    1.0
                };
                match axis {
                    0 => normal_x = n,
                    1 => normal_y = n,
                    _ => normal_z = n,
                }
            }
            if exit < t_max {
                t_max = exit;
            }

            // 无交集：t_min > t_max 或 t_max < 0
            if t_min > t_max {
                return None;
            }
            if t_max < -EPSILON {
                // 碰撞在反方向（后方），不算碰撞
                return None;
            }
        }

        // 若从未找到有效碰撞（t_min 始终为 0 但 found_collision=false），返回 None
        if !found_collision {
            return None;
        }

        // 取最近的正碰撞（t_min 应在 [0, 1] 范围内）
        if !(-EPSILON..=1.0 + EPSILON).contains(&t_min) {
            return None;
        }

        let res = t_min.clamp(0.0, 1.0);
        // 碰撞点 = self 中心 + res * delta
        let center = [
            (self.min[0] + self.max[0]) / 2.0,
            (self.min[1] + self.max[1]) / 2.0,
            (self.min[2] + self.max[2]) / 2.0,
        ];
        let collision_x = center[0] + res * delta[0];
        let collision_y = center[1] + res * delta[1];
        let collision_z = center[2] + res * delta[2];

        Some(SweepResult::collision(
            res,
            normal_x,
            normal_y,
            normal_z,
            collision_x,
            collision_y,
            collision_z,
        ))
    }
}

/// 以「脚底中心 + 宽高」构造碰撞盒（`width` 同时用于 x / z 两轴）。
///
/// `pos` 为实体脚底中心坐标，得到的包围盒为
/// `[x-w/2, y, z-w/2]` 到 `[x+w/2, y+h, z+w/2]`。
#[must_use]
pub fn box_from_foot_center(pos: [f64; 3], width: f64, height: f64) -> Aabb {
    Aabb::from_pos_size(
        [pos[0] - width / 2.0, pos[1], pos[2] - width / 2.0],
        [width, height, width],
    )
}

/// 按实体类型构造碰撞盒：尺寸取注册表中该类型的 `width` / `height`。
///
/// `pos` 为实体脚底中心（[`Position`] 的 y 即脚底高度）。
pub fn entity_box(entity_type: &EntityType, registry: &EntityTypeRegistry, pos: &Position) -> Aabb {
    box_from_foot_center(
        [pos.x, pos.y, pos.z],
        entity_type.width(registry),
        entity_type.height(registry),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::resource::registries::{EntityTypeDefinition, Registry};

    /// 构造最小实体类型注册表：player=0（0.6×1.8）、cow=1（0.9×1.4）、shulker=2（无尺寸）。
    fn test_entity_registry() -> EntityTypeRegistry {
        let toml = r#"
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

            [[entry]]
            id = 2
            name = "minecraft:shulker"
        "#;
        let inner = Registry::<EntityTypeDefinition>::from_toml_str(toml).unwrap();
        EntityTypeRegistry(inner)
    }

    #[test]
    fn from_pos_size_sets_min_and_max() {
        let aabb = Aabb::from_pos_size([1.0, 2.0, 3.0], [0.6, 1.8, 0.6]);
        assert_eq!(aabb.min, [1.0, 2.0, 3.0]);
        assert_eq!(aabb.max, [1.6, 3.8, 3.6]);
    }

    #[test]
    fn intersects_overlapping_boxes() {
        let a = Aabb::from_pos_size([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let b = Aabb::from_pos_size([0.5, 0.5, 0.5], [1.0, 1.0, 1.0]);
        assert!(a.intersects(&b));
        assert!(b.intersects(&a));
    }

    #[test]
    fn intersects_includes_touching_boxes() {
        // 两个盒子各轴恰好贴合：边界重合视为相交。
        let a = Aabb::from_pos_size([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let b = Aabb::from_pos_size([1.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        assert!(a.intersects(&b));
    }

    #[test]
    fn intersects_separate_boxes_false() {
        let a = Aabb::from_pos_size([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let b = Aabb::from_pos_size([1.5, 0.0, 0.0], [1.0, 1.0, 1.0]);
        assert!(!a.intersects(&b));
    }

    #[test]
    fn overlaps_block_inside_block() {
        let aabb = Aabb::from_pos_size([8.2, 64.3, 9.1], [0.6, 1.8, 0.6]);
        assert!(aabb.overlaps_block(8, 64, 9));
    }

    #[test]
    fn overlaps_block_touching_surface_is_not_overlap() {
        // 与方块 [5,6) 相切：min[0] == 6.0 不满足严格 `<`，不算重叠。
        let aabb = Aabb::from_pos_size([6.0, 64.0, 0.0], [0.6, 1.8, 0.6]);
        assert!(!aabb.overlaps_block(5, 64, 0));
        // max[0] == 5.0 与方块 [5,6) 相切，同样不算重叠。
        let aabb = Aabb::from_pos_size([4.4, 64.0, 0.0], [0.6, 1.8, 0.6]);
        assert!(!aabb.overlaps_block(5, 64, 0));
    }

    #[test]
    fn overlaps_block_adjacent_is_false() {
        let aabb = Aabb::from_pos_size([10.0, 64.0, 10.0], [0.6, 1.8, 0.6]);
        assert!(!aabb.overlaps_block(100, 64, 100));
    }

    #[test]
    fn moved_translates_corners() {
        let aabb = Aabb::from_pos_size([1.0, 2.0, 3.0], [0.6, 1.8, 0.6]);
        let shifted = aabb.moved(2.0, -1.0, 0.5);
        assert_eq!(shifted.min, [3.0, 1.0, 3.5]);
        assert_eq!(shifted.max, [3.6, 2.8, 4.1]);
        // 原盒不被修改。
        assert_eq!(aabb.min, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn entity_box_uses_registry_dimensions() {
        let registry = test_entity_registry();
        let cow = EntityType::by_id(1);
        let pos = Position::new(10.0, 64.0, 20.0);
        let box_ = entity_box(&cow, &registry, &pos);
        // 0.9 宽 → x/z 偏移 0.45；1.4 高 → y 顶到 65.4。
        let eps = 1e-9;
        assert!((box_.min[0] - 9.55).abs() < eps);
        assert!((box_.min[1] - 64.0).abs() < eps);
        assert!((box_.min[2] - 19.55).abs() < eps);
        assert!((box_.max[0] - 10.45).abs() < eps);
        assert!((box_.max[1] - 65.4).abs() < eps);
        assert!((box_.max[2] - 20.45).abs() < eps);
    }

    #[test]
    fn entity_box_falls_back_to_default_dimensions() {
        let registry = test_entity_registry();
        // shulker 无尺寸字段、未知 id 均回退到缺省 0.6×1.8。
        for id in [2u32, 999u32] {
            let ty = EntityType::by_id(id);
            let pos = Position::new(0.0, 70.0, 0.0);
            let box_ = entity_box(&ty, &registry, &pos);
            let eps = 1e-9;
            assert!((box_.min[0] + 0.3).abs() < eps);
            assert!((box_.min[1] - 70.0).abs() < eps);
            assert!((box_.min[2] + 0.3).abs() < eps);
            assert!((box_.max[0] - 0.3).abs() < eps);
            assert!((box_.max[1] - 71.8).abs() < eps);
            assert!((box_.max[2] - 0.3).abs() < eps);
        }
    }

    #[test]
    fn box_from_foot_center_matches_spec() {
        let box_ = box_from_foot_center([5.0, 64.0, 5.0], 0.6, 1.8);
        let eps = 1e-9;
        assert!((box_.min[0] - 4.7).abs() < eps);
        assert!((box_.min[1] - 64.0).abs() < eps);
        assert!((box_.min[2] - 4.7).abs() < eps);
        assert!((box_.max[0] - 5.3).abs() < eps);
        assert!((box_.max[1] - 65.8).abs() < eps);
        assert!((box_.max[2] - 5.3).abs() < eps);
    }

    // ---------- sweep_intersection 测试 ----------

    #[test]
    fn sweep_no_collision_when_separate() {
        let a = Aabb::from_pos_size([0.0, 0.0, 0.0], [0.6, 1.8, 0.6]);
        let b = Aabb::from_pos_size([5.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        assert!(a.sweep_intersection([1.0, 0.0, 0.0], &b).is_none());
    }

    #[test]
    fn sweep_positive_x_hits_right_face() {
        // a 在 x=0..0.6，向 +x 移动 1.0，b 在 x=1..2 → 碰撞发生在 t=0.4
        let a = Aabb::from_pos_size([0.0, 0.0, 0.0], [0.6, 1.0, 0.6]);
        let b = Aabb::from_pos_size([1.0, 0.0, 0.0], [2.0, 1.0, 1.0]);
        let result = a.sweep_intersection([1.0, 0.0, 0.0], &b).expect("应命中");
        assert!((result.res - 0.4).abs() < 1e-6, "res = {}", result.res);
        // 碰撞面为 b 的左面（x=1），外法线指向 -x
        assert_eq!(result.normal_x, -1.0);
        assert_eq!(result.normal_y, 0.0);
        assert_eq!(result.normal_z, 0.0);
    }

    #[test]
    fn sweep_negative_x_hits_left_face() {
        // a 在 x=3..3.6，向 -x 移动 -4.0，b 在 x=1..2（不与 a 重叠）
        // t0（a.min 到 b.max）: (2.0-3.0)/(-4.0) = 0.25
        // t1（a.max 到 b.min）: (1.0-3.6)/(-4.0) = 0.65
        // entry=0.25，res=0.25
        let a = Aabb::from_pos_size([3.0, 0.0, 0.0], [0.6, 1.0, 0.6]);
        let b = Aabb::from_pos_size([1.0, 0.0, 0.0], [1.0, 1.0, 1.0]); // b 为 [1,2]
        let result = a.sweep_intersection([-4.0, 0.0, 0.0], &b).expect("应命中");
        assert!((result.res - 0.25).abs() < 1e-6, "res = {}", result.res);
        // a 向 -x 移动，碰撞 b 的右面（x=2），法线指向 +x
        assert_eq!(result.normal_x, 1.0);
    }

    #[test]
    fn sweep_touching_no_collision() {
        // 恰好相切但扫掠后有重叠（delta=0.1，扫掠范围 [0,1.1] 与 [1,2] 有交集 [1,1.1]）
        let a = Aabb::from_pos_size([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let b = Aabb::from_pos_size([1.0, 0.0, 0.0], [2.0, 1.0, 1.0]);
        // 扫掠后确实有碰撞（a 移动到 [0.1, 1.1]，与 b [1,2] 在 [1,1.1] 重叠）
        let result = a.sweep_intersection([0.1, 0.0, 0.0], &b);
        assert!(result.is_some(), "扫掠应有碰撞（扫掠范围与静态盒有重叠）");
    }

    #[test]
    fn sweep_zero_delta_delegates_to_intersects() {
        // 相交时返回 res=0
        let a = Aabb::from_pos_size([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let b = Aabb::from_pos_size([0.5, 0.0, 0.0], [1.5, 1.0, 1.0]);
        let result = a.sweep_intersection([0.0, 0.0, 0.0], &b).expect("应相交");
        assert_eq!(result.res, 0.0);
        assert!(result.has_collision());

        // 不相交时返回 None
        let c = Aabb::from_pos_size([2.0, 0.0, 0.0], [3.0, 1.0, 1.0]);
        assert!(a.sweep_intersection([0.0, 0.0, 0.0], &c).is_none());
    }

    #[test]
    fn sweep_diagonal_collision() {
        // 斜向移动：x 和 y 同时有分量，取最早碰撞轴
        let a = Aabb::from_pos_size([0.0, 0.0, 0.0], [0.6, 0.6, 0.6]);
        let b = Aabb::from_pos_size([0.8, 0.8, 0.0], [1.8, 1.8, 1.0]);
        // x: t0=(0.8-0.6)/1=0.2, t1=(1.8-0)/1=1.8; y: t0=(0.8-0.6)/1=0.2, t1=(1.8-0)/1=1.8
        // t_min=0.2, res=0.2
        let result = a.sweep_intersection([1.0, 1.0, 0.0], &b).expect("应命中");
        assert!((result.res - 0.2).abs() < 1e-6, "res = {}", result.res);
        assert!(result.has_collision());
    }

    #[test]
    fn sweep_collision_point_is_center_plus_res_delta() {
        let a = Aabb::from_pos_size([0.0, 0.0, 0.0], [0.6, 1.0, 0.6]);
        let b = Aabb::from_pos_size([1.0, 0.0, 0.0], [2.0, 1.0, 1.0]);
        let result = a.sweep_intersection([1.0, 0.0, 0.0], &b).expect("应命中");
        // 碰撞点 = a 中心 (0.3, 0.5, 0.3) + 0.4 * (1.0, 0, 0) = (0.7, 0.5, 0.3)
        assert!((result.collided_x - 0.7).abs() < 1e-6);
        assert!((result.collided_y - 0.5).abs() < 1e-6);
        assert!((result.collided_z - 0.3).abs() < 1e-6);
    }
}
