// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 扫掠碰撞结果：记录一次射线-AABB 扫掠检测的碰撞信息。
//!
//! 对齐 Java [`net.minestom.server.collision.SweepResult`] 的语义，用于
//! [`Aabb::sweep_intersection`]、[`crate::physics::shape::Shape::intersect_swept`]
//! 等扫掠检测函数返回碰撞比例、法线与碰撞点。
//!
//! `res` 为命中比例（`[0, 1]`）：`0` 表示起点即在碰撞体内，`1` 表示全程无碰撞；
//! 无碰撞时 `res == f64::INFINITY`（由 [`NO_COLLISION`] 常量表达）。
//!
//! 变更标识符：`complete-collision-physics`（WS1）。
//! 见 `.specs/complete-collision-physics/spec.md`。

/// 扫掠碰撞结果。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SweepResult {
    /// 碰撞比例 `[0, 1]`：`0` = 起点即碰撞，`1` = 全程无碰撞；无碰撞时为 `f64::INFINITY`。
    pub res: f64,
    /// 碰撞面法线 X 分量（`-1` 左，`1` 右，`0` 无）。
    pub normal_x: f64,
    /// 碰撞面法线 Y 分量（`-1` 下，`1` 上，`0` 无）。
    pub normal_y: f64,
    /// 碰撞面法线 Z 分量（`-1` 前，`1` 后，`0` 无）。
    pub normal_z: f64,
    /// 碰撞点世界坐标 X。
    pub collided_x: f64,
    /// 碰撞点世界坐标 Y。
    pub collided_y: f64,
    /// 碰撞点世界坐标 Z。
    pub collided_z: f64,
}

impl Default for SweepResult {
    fn default() -> Self {
        Self::NO_COLLISION
    }
}

impl SweepResult {
    /// 无碰撞常量：`res == INFINITY`，法线与碰撞点均为零。
    pub const NO_COLLISION: Self = Self {
        res: f64::INFINITY,
        normal_x: 0.0,
        normal_y: 0.0,
        normal_z: 0.0,
        collided_x: 0.0,
        collided_y: 0.0,
        collided_z: 0.0,
    };

    /// 判断是否发生了碰撞（`res < INFINITY` 且有限）。
    #[must_use]
    pub fn has_collision(&self) -> bool {
        self.res.is_finite() && self.res < f64::INFINITY
    }

    /// 构造无碰撞结果。
    #[must_use]
    pub fn no_collision() -> Self {
        Self::NO_COLLISION
    }

    /// 构造有碰撞结果。
    #[must_use]
    pub fn collision(
        res: f64,
        normal_x: f64,
        normal_y: f64,
        normal_z: f64,
        x: f64,
        y: f64,
        z: f64,
    ) -> Self {
        Self {
            res,
            normal_x,
            normal_y,
            normal_z,
            collided_x: x,
            collided_y: y,
            collided_z: z,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn no_collision_has_infinity_res() {
        assert!(SweepResult::NO_COLLISION.res.is_infinite());
        assert!(!SweepResult::NO_COLLISION.has_collision());
    }

    #[test]
    fn no_collision_all_zeros() {
        let r = SweepResult::NO_COLLISION;
        assert_eq!(r.normal_x, 0.0);
        assert_eq!(r.normal_y, 0.0);
        assert_eq!(r.normal_z, 0.0);
        assert_eq!(r.collided_x, 0.0);
        assert_eq!(r.collided_y, 0.0);
        assert_eq!(r.collided_z, 0.0);
    }

    #[test]
    fn collision_constructor_sets_fields() {
        let r = SweepResult::collision(0.5, 1.0, 0.0, 0.0, 1.0, 2.0, 3.0);
        assert!((r.res - 0.5).abs() < 1e-9);
        assert_eq!(r.normal_x, 1.0);
        assert_eq!(r.collided_x, 1.0);
        assert!(r.has_collision());
    }

    #[test]
    fn default_is_no_collision() {
        let r = SweepResult::default();
        assert!(!r.has_collision());
        assert_eq!(r, SweepResult::NO_COLLISION);
    }

    #[test]
    fn no_collision_method() {
        let r = SweepResult::no_collision();
        assert!(!r.has_collision());
        assert_eq!(r, SweepResult::NO_COLLISION);
    }
}
