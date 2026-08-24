// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 方块形状抽象：统一「实心方块 / 空 / 多个 AABB 合并」三种碰撞形态。
//!
//! [`Shape`] 对应 Java 侧 `Shape` 的 `AABB` / `EMPTY` / `merged` 三种形态，
//! 但按本任务范围简化：不承载朝向、不区分「占据 / 碰撞」两类盒子，仅提供
//! 与单个 [`Aabb`] 的相交判定、方块重叠判定与平移运算。[`Shape::merged`]
//! 把多个 AABB 合并成一个形状；空输入退化为 [`Shape::Empty`]。
//!
//! 变更标识符：`complete-partial-framework-capabilities`（T4 碰撞增强）。
//! 见 `.specs/complete-partial-framework-capabilities/spec.md`。

use super::Aabb;
use super::sweep_result::SweepResult;

/// 方块碰撞形状：单位 / 空 / 多盒子合并。
///
/// - [`Shape::Aabb`]：单个轴对齐包围盒（本阶段实心方块即单位盒子）。
/// - [`Shape::Empty`]：空气等不可碰撞形态，与任何盒子恒不相交。
/// - [`Shape::Merged`]：多个包围盒的组合，与任一子盒相交即视为相交。
///
/// 本阶段 `Merged` 仅做平移与查询，不对相邻盒子做几何合并（v1 边界）。
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// 单个轴对齐包围盒。
    Aabb(Aabb),
    /// 空形状（空气 / 液体等不可碰撞形态）。
    Empty,
    /// 多个包围盒的合并组合。
    Merged(Vec<Aabb>),
}

impl Shape {
    /// 是否与指定包围盒相交（含相切）。
    ///
    /// `Empty` 恒不相交；`Merged` 只要任一子盒相交即为相交。
    #[must_use]
    pub fn intersects(&self, other: &Aabb) -> bool {
        match self {
            Shape::Aabb(aabb) => aabb.intersects(other),
            Shape::Empty => false,
            Shape::Merged(boxes) => boxes.iter().any(|aabb| aabb.intersects(other)),
        }
    }

    /// 是否与指定单位方块 `[x, x+1) × [y, y+1) × [z, z+1)` 重叠。
    ///
    /// 与 [`Aabb::overlaps_block`] 一致采用严格相交（相切不算重叠）。
    #[must_use]
    pub fn overlaps_block(&self, x: i32, y: i32, z: i32) -> bool {
        match self {
            Shape::Aabb(aabb) => aabb.overlaps_block(x, y, z),
            Shape::Empty => false,
            Shape::Merged(boxes) => boxes.iter().any(|aabb| aabb.overlaps_block(x, y, z)),
        }
    }

    /// 平移后的副本：所有子盒分别加上 `(dx, dy, dz)`。
    #[must_use]
    pub fn moved(&self, dx: f64, dy: f64, dz: f64) -> Self {
        match self {
            Shape::Aabb(aabb) => Shape::Aabb(aabb.moved(dx, dy, dz)),
            Shape::Empty => Shape::Empty,
            Shape::Merged(boxes) => {
                Shape::Merged(boxes.iter().map(|aabb| aabb.moved(dx, dy, dz)).collect())
            }
        }
    }

    /// 是否为「空」形状（不可碰撞）。
    ///
    /// `Aabb` 恒为 false；`Merged` 在子盒为空时视为空（正常路径由
    /// [`Shape::merged`] 保证不产生空 `Merged`，此处仅作防御）。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            Shape::Aabb(_) => false,
            Shape::Empty => true,
            Shape::Merged(boxes) => boxes.is_empty(),
        }
    }

    /// 将多个包围盒合并为一个 [`Shape`]。
    ///
    /// 空输入返回 [`Shape::Empty`]；否则返回 [`Shape::Merged`]。本阶段不做
    /// 相邻盒子的几何合并（v1 边界，见模块文档）。
    #[must_use]
    pub fn merged(iter: impl IntoIterator<Item = Aabb>) -> Self {
        let boxes: Vec<Aabb> = iter.into_iter().collect();
        if boxes.is_empty() {
            Shape::Empty
        } else {
            Shape::Merged(boxes)
        }
    }

    /// 检测本形状沿射线 `ray_start` → `ray_start + ray_direction` 扫掠是否与
    /// 移动 AABB `moving` 相交（对齐 Java `Shape.intersectBoxSwept`）。
    ///
    /// `shape_pos` 为本形状的世界坐标原点（用于将形状平移到世界空间）。
    /// 命中时更新 `final_result` 为最近碰撞（最小 `res`）。
    ///
    /// - 返回 `true`：有碰撞，`final_result` 已更新。
    /// - 返回 `false`：无碰撞，`final_result` 不变。
    #[must_use]
    pub fn intersect_swept(
        &self,
        _ray_start: [f64; 3],
        ray_direction: [f64; 3],
        shape_pos: [f64; 3],
        moving: &Aabb,
        final_result: &mut SweepResult,
    ) -> bool {
        match self {
            Shape::Empty => false,
            Shape::Aabb(aabb) => {
                // 将静止 AABB 平移到世界空间
                let world_aabb = aabb.moved(shape_pos[0], shape_pos[1], shape_pos[2]);
                match moving.sweep_intersection(ray_direction, &world_aabb) {
                    Some(r) if r.res < final_result.res => {
                        *final_result = r;
                        true
                    }
                    Some(_) => {
                        // 已有更近的碰撞，不更新
                        false
                    }
                    None => false,
                }
            }
            Shape::Merged(boxes) => {
                let mut hit = false;
                for box_ in boxes {
                    let world_aabb = box_.moved(shape_pos[0], shape_pos[1], shape_pos[2]);
                    if let Some(r) = moving.sweep_intersection(ray_direction, &world_aabb)
                        && r.res < final_result.res
                    {
                        *final_result = r;
                        hit = true;
                    }
                }
                hit
            }
        }
    }
}

impl From<Aabb> for Shape {
    fn from(aabb: Aabb) -> Self {
        Shape::Aabb(aabb)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    /// 测试用单位方块盒子 `[x, x+1) × [y, y+1) × [z, z+1)`。
    fn unit_at(x: f64, y: f64, z: f64) -> Aabb {
        Aabb::from_pos_size([x, y, z], [1.0, 1.0, 1.0])
    }

    #[test]
    fn aabb_variant_intersects() {
        let shape = Shape::Aabb(unit_at(0.0, 0.0, 0.0));
        let probe = unit_at(0.5, 0.5, 0.5);
        assert!(shape.intersects(&probe));
        let far = unit_at(5.0, 5.0, 5.0);
        assert!(!shape.intersects(&far));
    }

    #[test]
    fn empty_never_intersects() {
        let shape = Shape::Empty;
        assert!(shape.is_empty());
        assert!(!shape.intersects(&unit_at(0.0, 0.0, 0.0)));
        assert!(!shape.overlaps_block(0, 0, 0));
        // 平移后仍为空。
        assert!(shape.moved(1.0, 2.0, 3.0).is_empty());
    }

    #[test]
    fn merged_intersects_if_any_subbox_hits() {
        let shape = Shape::merged([unit_at(0.0, 0.0, 0.0), unit_at(10.0, 0.0, 0.0)]);
        // 命中第一个子盒。
        assert!(shape.intersects(&unit_at(0.5, 0.5, 0.5)));
        // 命中第二个子盒。
        assert!(shape.intersects(&unit_at(10.5, 0.5, 0.5)));
        // 都不命中（两盒之间的缝隙）。
        assert!(!shape.intersects(&unit_at(5.0, 0.0, 0.0)));
        // overlaps_block 同语义。
        assert!(shape.overlaps_block(0, 0, 0));
        assert!(shape.overlaps_block(10, 0, 0));
        assert!(!shape.overlaps_block(5, 0, 0));
        assert!(!shape.is_empty());
    }

    #[test]
    fn merged_with_empty_input_returns_empty() {
        let shape = Shape::merged(std::iter::empty::<Aabb>());
        assert_eq!(shape, Shape::Empty);
        assert!(shape.is_empty());
    }

    #[test]
    fn merged_moved_translates_each_subbox() {
        let shape = Shape::merged([unit_at(0.0, 0.0, 0.0), unit_at(10.0, 0.0, 0.0)]);
        let shifted = shape.moved(2.0, -1.0, 0.5);
        match shifted {
            Shape::Merged(boxes) => {
                assert_eq!(boxes.len(), 2);
                assert_eq!(boxes[0].min, [2.0, -1.0, 0.5]);
                assert_eq!(boxes[1].min, [12.0, -1.0, 0.5]);
            }
            other => panic!("预期 Merged，得到 {other:?}"),
        }
    }

    #[test]
    fn moved_translates_variants() {
        let aabb = Shape::Aabb(unit_at(0.0, 0.0, 0.0)).moved(1.0, 2.0, 3.0);
        match aabb {
            Shape::Aabb(b) => assert_eq!(b.min, [1.0, 2.0, 3.0]),
            other => panic!("预期 Aabb，得到 {other:?}"),
        }
    }

    #[test]
    fn is_empty_only_true_for_empty() {
        assert!(Shape::Empty.is_empty());
        assert!(!Shape::Aabb(unit_at(0.0, 0.0, 0.0)).is_empty());
        assert!(!Shape::merged([unit_at(0.0, 0.0, 0.0)]).is_empty());
    }

    #[test]
    fn from_aabb_conversion() {
        let shape = Shape::from(unit_at(3.0, 4.0, 5.0));
        assert_eq!(shape, Shape::Aabb(unit_at(3.0, 4.0, 5.0)));
        assert!(shape.overlaps_block(3, 4, 5));
    }

    // ---------- intersect_swept 测试 ----------

    #[test]
    fn empty_never_hits() {
        let mut result = SweepResult::NO_COLLISION;
        assert!(!Shape::Empty.intersect_swept(
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            &Aabb::from_pos_size([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]),
            &mut result
        ));
        assert!(result.res.is_infinite());
    }

    #[test]
    fn aabb_variant_hits_when_intersected() {
        let moving = Aabb::from_pos_size([0.0, 0.0, 0.0], [0.5, 0.5, 0.5]);
        let shape = Shape::Aabb(unit_at(1.0, 0.0, 0.0));
        let mut result = SweepResult::NO_COLLISION;
        // 从 (0.25,0.25,0.25) 向 +x 扫掠，目标在 x=1..2
        let hit = shape.intersect_swept(
            [0.25, 0.25, 0.25],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            &moving,
            &mut result,
        );
        assert!(hit);
        assert!(result.has_collision());
    }

    #[test]
    fn merged_hits_first_subbox() {
        let moving = Aabb::from_pos_size([0.0, 0.0, 0.0], [0.5, 0.5, 0.5]);
        let shape = Shape::Merged(vec![unit_at(1.0, 0.0, 0.0), unit_at(5.0, 0.0, 0.0)]);
        let mut result = SweepResult::NO_COLLISION;
        let hit = shape.intersect_swept(
            [0.25, 0.25, 0.25],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            &moving,
            &mut result,
        );
        assert!(hit);
        // 命中第一个子盒（x=1），而非第二个（x=5）
        assert!((result.collided_x - 0.75).abs() < 1e-6);
    }

    #[test]
    fn merged_no_hit_returns_false() {
        let moving = Aabb::from_pos_size([0.0, 0.0, 0.0], [0.5, 0.5, 0.5]);
        let shape = Shape::Merged(vec![unit_at(5.0, 0.0, 0.0)]);
        let mut result = SweepResult::NO_COLLISION;
        // 移动方向远离目标
        let hit = shape.intersect_swept(
            [0.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            &moving,
            &mut result,
        );
        assert!(!hit);
        assert!(result.res.is_infinite());
    }
}
