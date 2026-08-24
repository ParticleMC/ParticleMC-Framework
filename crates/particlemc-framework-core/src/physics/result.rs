//! 物理模拟结果：封装一次完整运动物理（方块碰撞 + 空气动力学）的输出。
//!
//! 对齐 Java [`net.minestom.server.collision.PhysicsResult`]。由
//! [`crate::physics::utils::simulate_movement`] 返回，供应用层查询碰撞轴、
//! 地面状态与新速度。
//!
//! 变更标识符：`complete-collision-physics`（WS5）。
//! 见 `.specs/complete-collision-physics/spec.md`。

use crate::physics::SweepResult;

/// 物理模拟结果。
#[derive(Copy, Clone, Debug)]
pub struct PhysicsResult {
    /// 碰撞后新位置（世界坐标 `[x, y, z]`）。
    pub new_position: [f64; 3],
    /// 碰撞后新速度（方块/秒）`[x, y, z]`。
    pub new_velocity: [f64; 3],
    /// 是否在地面（Y 轴碰撞且原始 Y 速度向下）。
    pub is_on_ground: bool,
    /// X 轴是否发生碰撞。
    pub collision_x: bool,
    /// Y 轴是否发生碰撞。
    pub collision_y: bool,
    /// Z 轴是否发生碰撞。
    pub collision_z: bool,
    /// 原始位移向量（用于缓存比对）。
    pub original_delta: [f64; 3],
    /// 总碰撞标志（任一轴碰撞为 true）。
    pub has_collision: bool,
    /// 扫掠碰撞详情（最近碰撞的法线与比例）。
    pub res: SweepResult,
}

impl PhysicsResult {
    /// 构造无碰撞的自由移动结果。
    #[must_use]
    pub fn free_move(position: [f64; 3], velocity: [f64; 3], original_delta: [f64; 3]) -> Self {
        Self {
            new_position: position,
            new_velocity: velocity,
            is_on_ground: false,
            collision_x: false,
            collision_y: false,
            collision_z: false,
            original_delta,
            has_collision: false,
            res: SweepResult::NO_COLLISION,
        }
    }

    /// 构造有碰撞的结果。
    #[must_use]
    pub fn with_collision(
        new_position: [f64; 3],
        new_velocity: [f64; 3],
        collision_x: bool,
        collision_y: bool,
        collision_z: bool,
        original_delta: [f64; 3],
        res: SweepResult,
    ) -> Self {
        let has_collision = collision_x || collision_y || collision_z;
        let is_on_ground = collision_y && original_delta[1] < 0.0;
        Self {
            new_position,
            new_velocity,
            is_on_ground,
            collision_x,
            collision_y,
            collision_z,
            original_delta,
            has_collision,
            res,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn free_move_has_no_collision() {
        let r = PhysicsResult::free_move([0.0, 64.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
        assert!(!r.has_collision);
        assert!(!r.collision_x && !r.collision_y && !r.collision_z);
        assert!(!r.is_on_ground);
        assert_eq!(r.new_position, [0.0, 64.0, 0.0]);
        assert_eq!(r.new_velocity, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn with_collision_sets_flags() {
        let res = SweepResult::collision(0.5, 1.0, 0.0, 0.0, 1.0, 2.0, 3.0);
        let r = PhysicsResult::with_collision(
            [1.0, 2.0, 3.0],
            [0.0, 0.0, 0.0],
            true,
            false,
            false,
            [1.0, 0.0, 0.0],
            res,
        );
        assert!(r.has_collision);
        assert!(r.collision_x);
        assert!(!r.collision_y);
        assert!(!r.is_on_ground);
    }

    #[test]
    fn landing_sets_is_on_ground() {
        // 下落撞地：原始 y 速度向下（original_delta.y < 0），collision_y = true
        let res = SweepResult::collision(0.0, 0.0, 1.0, 0.0, 0.0, 65.0, 0.0);
        let r = PhysicsResult::with_collision(
            [0.0, 65.0, 0.0],
            [0.0, 0.0, 0.0],
            false,
            true,
            false,
            [0.0, -1.0, 0.0],
            res,
        );
        assert!(r.is_on_ground);
        assert!(r.collision_y);
    }

    #[test]
    fn hitting_ceiling_does_not_set_on_ground() {
        // 上跳撞天花板：original_delta.y > 0，collision_y = true
        let res = SweepResult::collision(1.0, 0.0, -1.0, 0.0, 0.0, 70.0, 0.0);
        let r = PhysicsResult::with_collision(
            [0.0, 70.0, 0.0],
            [0.0, 0.0, 0.0],
            false,
            true,
            false,
            [0.0, 1.0, 0.0],
            res,
        );
        assert!(!r.is_on_ground);
        assert!(r.collision_y);
    }
}
