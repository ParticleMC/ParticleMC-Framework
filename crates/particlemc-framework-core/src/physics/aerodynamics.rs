//! 空气动力学属性：描述实体受到的重力与空气阻力。
//!
//! 对齐 Java [`net.minestom.server.collision.Aerodynamics`]。各字段为不可变
//! 记录，通过 `with_*` 方法返回新实例，保持值类型语义。
//!
//! 默认值对齐 vanilla Minecraft：重力 `0.08` 方块/tick²，水平/竖直阻力 `0.98`。
//!
//! 变更标识符：`complete-collision-physics`（WS4）。
//! 见 `.specs/complete-collision-physics/spec.md`。

/// 实体空气动力学属性。
///
/// - `gravity`：重力加速度（方块/tick²），方向由调用方约定（通常向下为负）。
/// - `horizontal_drag`：水平空气阻力系数，每 tick `vel *= drag`。
/// - `vertical_drag`：竖直空气阻力系数，每 tick `vel *= drag`。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Aerodynamics {
    /// 重力加速度（方块/tick²）。
    pub gravity: f64,
    /// 水平空气阻力系数（`[0, 1]`，越小阻力越大）。
    pub horizontal_drag: f64,
    /// 竖直空气阻力系数（`[0, 1]`，越小阻力越大）。
    pub vertical_drag: f64,
}

impl Default for Aerodynamics {
    fn default() -> Self {
        Self {
            gravity: 0.08,
            horizontal_drag: 0.98,
            vertical_drag: 0.98,
        }
    }
}

impl Aerodynamics {
    /// 以三字段显式构造。
    #[must_use]
    pub fn new(gravity: f64, horizontal_drag: f64, vertical_drag: f64) -> Self {
        Self {
            gravity,
            horizontal_drag,
            vertical_drag,
        }
    }

    /// 返回新实例，替换重力加速度。
    #[must_use]
    pub fn with_gravity(self, gravity: f64) -> Self {
        Self { gravity, ..self }
    }

    /// 返回新实例，替换水平空气阻力系数。
    #[must_use]
    pub fn with_horizontal_drag(self, drag: f64) -> Self {
        Self {
            horizontal_drag: drag,
            ..self
        }
    }

    /// 返回新实例，替换竖直空气阻力系数。
    #[must_use]
    pub fn with_vertical_drag(self, drag: f64) -> Self {
        Self {
            vertical_drag: drag,
            ..self
        }
    }

    /// 返回新实例，同时替换水平和竖直阻力系数。
    #[must_use]
    pub fn with_drag(self, horizontal: f64, vertical: f64) -> Self {
        Self {
            horizontal_drag: horizontal,
            vertical_drag: vertical,
            ..self
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn default_values_match_vanilla() {
        let a = Aerodynamics::default();
        assert!((a.gravity - 0.08).abs() < 1e-9);
        assert!((a.horizontal_drag - 0.98).abs() < 1e-9);
        assert!((a.vertical_drag - 0.98).abs() < 1e-9);
    }

    #[test]
    fn with_gravity_changes_only_gravity() {
        let base = Aerodynamics::new(0.08, 0.98, 0.98);
        let modified = base.with_gravity(0.0);
        assert_eq!(modified.gravity, 0.0);
        assert_eq!(modified.horizontal_drag, 0.98);
        assert_eq!(modified.vertical_drag, 0.98);
    }

    #[test]
    fn with_horizontal_drag_changes_only_horizontal() {
        let base = Aerodynamics::default();
        let modified = base.with_horizontal_drag(0.5);
        assert_eq!(modified.gravity, 0.08);
        assert_eq!(modified.horizontal_drag, 0.5);
        assert_eq!(modified.vertical_drag, 0.98);
    }

    #[test]
    fn with_vertical_drag_changes_only_vertical() {
        let base = Aerodynamics::default();
        let modified = base.with_vertical_drag(0.5);
        assert_eq!(modified.gravity, 0.08);
        assert_eq!(modified.horizontal_drag, 0.98);
        assert_eq!(modified.vertical_drag, 0.5);
    }

    #[test]
    fn with_drag_changes_both() {
        let base = Aerodynamics::default();
        let modified = base.with_drag(0.9, 0.9);
        assert_eq!(modified.horizontal_drag, 0.9);
        assert_eq!(modified.vertical_drag, 0.9);
        assert_eq!(modified.gravity, 0.08);
    }

    #[test]
    fn zero_gravity_no_gravity_entity() {
        let a = Aerodynamics::default().with_gravity(0.0);
        assert_eq!(a.gravity, 0.0);
    }
}
