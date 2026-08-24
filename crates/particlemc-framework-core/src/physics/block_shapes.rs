// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 方块形状表：将方块名映射为局部坐标下的多 AABB 子盒。
//!
//! 本模块是 `complete-framework-gaps` WS3 的核心数据：由方块名解析其碰撞形状，
//! 供 [`crate::physics::block_collision::block_shape`] 构造
//! [`crate::physics::shape::Shape`]。形状以「单位立方体 `[0,1)³` 内的局部坐标」
//! 表达，支持 slab / stair 等非单位方块（如台阶下半、上半、楼梯多盒）。
//!
//! 变更标识符：`complete-framework-gaps`（WS3）。冻结接口：`BlockShapeTable`。
//! 子系统实现见 [`.specs/complete-framework-gaps/spec.md`]。
//!
//! # 形状约定（v1 简化边界）
//! vanilla 用方块状态属性（如 slab 的 `type = top/bottom`）决定形状；当前注册表
//! 不承载逐状态形状，`v1` 以**名称后缀启发式**近似，文档记为已知弱化：
//! - 以 `_slab` 结尾：下半 slab，占据 `[0, 0.5)`（顶面在 `y = 0.5`）。
//! - 以 `_slab_top` 结尾：上半 slab，占据 `[0.5, 1)`（仅上沿 `[0.5, 1)` 阻挡）。
//! - 名称含 `stairs`：楼梯，两个子盒（下半整块 + 上半后半个）近似。
//! - 其余实心方块：单位立方体 `[0,1)³`。

use super::Aabb;

/// 单个子盒的六元组布局：`[min_x, min_y, min_z, max_x, max_y, max_z]`。
///
/// 各分量 ∈ `[0, 1]`，表示单位立方体 `[0,1)³` 内的局部坐标。该类型是
/// 冻结接口 `BlockShapeTable` 的返回元素类型，禁止改布局。
pub type Box6 = [f64; 6];

/// 由方块名解析其碰撞子盒列表（局部坐标 `[0,1)³` 内）。
///
/// 返回 `None` 表示空气 / 透明（不可碰撞）；返回单盒可能为普通实心方块
/// （单位立方体）或 slab 等非单位形状；返回多盒即 stair 等多盒形状。
/// 调用方据几何判定构造 [`crate::physics::shape::Shape`]：单位单盒 → `Aabb`，
/// 非单位单盒 / 多盒 → `Merged`（`Empty` 对应 `None`）。
///
/// 形状语义见模块文档「形状约定」。后续若注册表承载逐状态形状字段，
/// 可在此前置查表，无需改动调用方。
#[must_use]
pub fn shape_boxes(block_name: &str) -> Option<Vec<Box6>> {
    if block_name == "minecraft:air" {
        return None;
    }
    // 下半 slab：占据 [0, 0.5)（顶面在 y = 0.5）。
    let lower_slab: Box6 = [0.0, 0.0, 0.0, 1.0, 0.5, 1.0];
    // 上半 slab：占据 [0.5, 1)（仅上沿阻挡）。
    let upper_slab: Box6 = [0.0, 0.5, 0.0, 1.0, 1.0, 1.0];
    // 楼梯近似：下半整块 + 上半后半个（z ≥ 0.5）。
    let stair_lower: Box6 = [0.0, 0.0, 0.0, 1.0, 0.5, 1.0];
    let stair_upper: Box6 = [0.0, 0.5, 0.5, 1.0, 1.0, 1.0];

    if block_name.ends_with("_slab_top") {
        Some(vec![upper_slab])
    } else if block_name.ends_with("_slab") {
        Some(vec![lower_slab])
    } else if block_name.contains("stairs") {
        Some(vec![stair_lower, stair_upper])
    } else {
        // 普通实心方块：单位立方体。
        Some(vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]])
    }
}

/// 将六元组子盒转为 [`Aabb`]。
///
/// 输入越界分量会被钳制到 `[0, 1]`，保证形状几何有效（不出现负尺寸或越界）。
#[must_use]
pub fn box6_to_aabb(box6: &Box6) -> Aabb {
    let min_x = box6[0].clamp(0.0, 1.0);
    let min_y = box6[1].clamp(0.0, 1.0);
    let min_z = box6[2].clamp(0.0, 1.0);
    let max_x = box6[3].clamp(0.0, 1.0);
    let max_y = box6[4].clamp(0.0, 1.0);
    let max_z = box6[5].clamp(0.0, 1.0);
    let size_x = (max_x - min_x).max(0.0);
    let size_y = (max_y - min_y).max(0.0);
    let size_z = (max_z - min_z).max(0.0);
    Aabb::from_pos_size([min_x, min_y, min_z], [size_x, size_y, size_z])
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn air_is_none() {
        assert!(shape_boxes("minecraft:air").is_none());
    }

    #[test]
    fn solid_is_unit_box() {
        let boxes = shape_boxes("minecraft:stone").expect("stone 应有形状");
        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0], [0.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn lower_slab_occupies_bottom_half() {
        let boxes = shape_boxes("minecraft:oak_slab").expect("slab 应有形状");
        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0], [0.0, 0.0, 0.0, 1.0, 0.5, 1.0]);
    }

    #[test]
    fn upper_slab_occupies_top_half() {
        let boxes = shape_boxes("minecraft:oak_slab_top").expect("upper slab 应有形状");
        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0], [0.0, 0.5, 0.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn stair_has_two_boxes() {
        let boxes = shape_boxes("minecraft:oak_stairs").expect("stairs 应有形状");
        assert_eq!(boxes.len(), 2);
        assert_eq!(boxes[0], [0.0, 0.0, 0.0, 1.0, 0.5, 1.0]);
        assert_eq!(boxes[1], [0.0, 0.5, 0.5, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn box6_to_aabb_clamps_out_of_range() {
        let aabb = box6_to_aabb(&[-1.0, 2.0, 0.0, 2.0, -1.0, 0.5]);
        assert_eq!(aabb.min, [0.0, 1.0, 0.0]);
    }
}
