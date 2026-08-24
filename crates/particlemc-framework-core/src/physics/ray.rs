// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 射线与体素遍历：沿射线逐方块推进的 DDA 射线检测。
//!
//! [`Ray`] 由原点和方向定义（方向为零向量视为非法，构造返回 `None`）。
//! [`raycast`] 以 Amanatides & Woo 体素遍历从起点所在方块开始沿方向逐块推进，
//! 收集实心方块坐标并返回，直到命中 / 超距 / 越界终止（语义对齐 Java
//! `RayUtils` 的「首个实心方块即命中」行为）。
//!
//! 变更标识符：`complete-partial-framework-capabilities`（T4 碰撞增强）。
//! 见 `.specs/complete-partial-framework-capabilities/spec.md`。

/// 体素遍历的防御性步数上限：正常路径由 `max_distance` 截断不会触及，
/// 仅用于兜底方向分量极小等病态输入，避免无限循环。
const MAX_STEPS: usize = 1_000_000;

/// 射线：原点和方向向量。
#[derive(Clone, Debug, PartialEq)]
pub struct Ray {
    origin: [f64; 3],
    direction: [f64; 3],
}

impl Ray {
    /// 构造射线。
    ///
    /// 方向为零向量时返回 `None`：退化的射线无法推进，调用方需自行处理
    /// （本 API 选择「返回 None」而非 panic，由调用方决定取舍）。
    pub fn new(origin: [f64; 3], direction: [f64; 3]) -> Option<Self> {
        if direction == [0.0, 0.0, 0.0] {
            return None;
        }
        Some(Self { origin, direction })
    }
}

/// 浮点向下取整并安全转换为 `i32`。
///
/// 先钳制到 `i32` 可表示范围再 `as` 缩窄（超出部分饱和截断），避免越界值
/// 经裸 `as` 转换产生未定义行为；正常坐标远在范围内，行为等同直接取整。
fn floor_i32(v: f64) -> i32 {
    v.floor().clamp(i32::MIN as f64, i32::MAX as f64) as i32
}

/// 沿射线做体素遍历，返回途经的实心方块坐标序列。
///
/// - 从射线起点所在方块开始（含起点方块本身）逐块推进。
/// - 对每个途经方块调用 `is_solid(x, y, z)`；**首个实心方块即「命中」**，
///   将其坐标加入结果后停止（对齐 Java `RayUtils` 语义）。
/// - 终止条件：命中实心方块、跨过 `max_distance`、或坐标溢出（越界）。
/// - `is_solid` 恒为 false 时遍历至超距，返回空序列。
///
/// `max_distance` 为负或 NaN 时返回空序列。返回值按遍历先后排序。
#[must_use]
pub fn raycast(
    ray: &Ray,
    max_distance: f64,
    is_solid: impl Fn(i32, i32, i32) -> bool,
) -> Vec<(i32, i32, i32)> {
    let mut hits = Vec::new();
    if max_distance.is_nan() || max_distance < 0.0 {
        return hits;
    }

    let origin = ray.origin;
    let dir = ray.direction;
    // 当前方块：射线起点所在方块。
    let mut block = [
        floor_i32(origin[0]),
        floor_i32(origin[1]),
        floor_i32(origin[2]),
    ];

    // DDA 状态：每轴步进方向、跨过一个方块所需的参数距离、到下一边界的参数距离。
    // 方向分量为 0 的轴不推进（t_max 保持 +∞，永不成为最小轴）。
    let mut step = [0i32; 3];
    let mut t_delta = [f64::INFINITY; 3];
    let mut t_max = [f64::INFINITY; 3];
    for i in 0..3 {
        if dir[i] != 0.0 {
            step[i] = if dir[i] > 0.0 { 1 } else { -1 };
            t_delta[i] = (1.0 / dir[i]).abs();
            let current = floor_i32(origin[i]) as f64;
            t_max[i] = if dir[i] > 0.0 {
                (current + 1.0 - origin[i]) / dir[i]
            } else {
                (origin[i] - current) / dir[i].abs()
            };
        }
    }

    for _ in 0..MAX_STEPS {
        if is_solid(block[0], block[1], block[2]) {
            hits.push((block[0], block[1], block[2]));
            break;
        }
        // 选择参数距离最小的轴作为下一次跨边界的方向（tie-break：x → y → z）。
        let axis = if t_max[0] <= t_max[1] && t_max[0] <= t_max[2] {
            0
        } else if t_max[1] <= t_max[2] {
            1
        } else {
            2
        };
        // 下一方块已超出最大距离：终止。
        if t_max[axis] > max_distance {
            break;
        }
        // 推进到相邻方块；坐标溢出（越界）时终止，防止 i32 回绕。
        match block[axis].checked_add(step[axis]) {
            Some(next) => block[axis] = next,
            None => break,
        }
        t_max[axis] += t_delta[axis];
    }

    hits
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    /// 按预置的实心方块集合构造判定闭包。
    fn solid_at(blocks: &[(i32, i32, i32)]) -> impl Fn(i32, i32, i32) -> bool + '_ {
        move |x, y, z| {
            blocks
                .iter()
                .any(|&(bx, by, bz)| bx == x && by == y && bz == z)
        }
    }

    #[test]
    fn new_rejects_zero_direction() {
        assert!(Ray::new([0.5, 0.5, 0.5], [1.0, 0.0, 0.0]).is_some());
        assert!(Ray::new([0.5, 0.5, 0.5], [0.0, 0.0, 0.0]).is_none());
    }

    #[test]
    fn raycast_positive_x_hits_first_solid_block() {
        // 从 (0.5,0.5,0.5) 向 +x：起点块 (0,0,0) 为空气，(1,0,0) 实心 → 命中。
        let ray = Ray::new([0.5, 0.5, 0.5], [1.0, 0.0, 0.0]).unwrap();
        let solid = solid_at(&[(1, 0, 0)]);
        assert_eq!(raycast(&ray, 10.0, solid), vec![(1, 0, 0)]);
    }

    #[test]
    fn raycast_starts_inside_solid_hits_start_block() {
        // 起点方块本身就是实心：立即命中起点块。
        let ray = Ray::new([0.5, 0.5, 0.5], [1.0, 0.0, 0.0]).unwrap();
        let solid = solid_at(&[(0, 0, 0)]);
        assert_eq!(raycast(&ray, 10.0, solid), vec![(0, 0, 0)]);
    }

    #[test]
    fn raycast_diagonal_visits_intermediate_blocks() {
        // 斜向 45°：DDA 依次经过 (1,0,0)、(1,1,0)、(1,1,1)，最后一个实心。
        let ray = Ray::new([0.5, 0.5, 0.5], [1.0, 1.0, 1.0]).unwrap();
        let solid = solid_at(&[(1, 1, 1)]);
        assert_eq!(raycast(&ray, 10.0, solid), vec![(1, 1, 1)]);
    }

    #[test]
    fn raycast_diagonal_hits_first_solid_along_path() {
        // 斜向路径上先遇到 (1,1,0) 实心：命中它而非更远的 (1,1,1)。
        let ray = Ray::new([0.5, 0.5, 0.5], [1.0, 1.0, 1.0]).unwrap();
        let solid = solid_at(&[(1, 1, 0), (1, 1, 1)]);
        assert_eq!(raycast(&ray, 10.0, solid), vec![(1, 1, 0)]);
    }

    #[test]
    fn raycast_stops_beyond_max_distance() {
        // max_distance=0.3 < 到达 (1,0,0) 的 0.5 距离：超距终止，无命中。
        let ray = Ray::new([0.5, 0.5, 0.5], [1.0, 0.0, 0.0]).unwrap();
        let solid = solid_at(&[(1, 0, 0)]);
        assert!(raycast(&ray, 0.3, solid).is_empty());
    }

    #[test]
    fn raycast_negative_distance_returns_empty() {
        let ray = Ray::new([0.5, 0.5, 0.5], [1.0, 0.0, 0.0]).unwrap();
        let solid = solid_at(&[(0, 0, 0)]);
        assert!(raycast(&ray, -1.0, solid).is_empty());
    }

    #[test]
    fn raycast_no_solid_returns_empty() {
        // is_solid 恒 false：遍历至超距，返回空序列。
        let ray = Ray::new([0.5, 0.5, 0.5], [1.0, 0.0, 0.0]).unwrap();
        let solid = |_: i32, _: i32, _: i32| false;
        assert!(raycast(&ray, 10.0, solid).is_empty());
    }

    #[test]
    fn raycast_negative_axis_direction() {
        // 向 -x：起点 (1,0,0) 空气，(0,0,0) 实心 → 命中起点西侧方块。
        let ray = Ray::new([1.5, 0.5, 0.5], [-1.0, 0.0, 0.0]).unwrap();
        let solid = solid_at(&[(0, 0, 0)]);
        assert_eq!(raycast(&ray, 10.0, solid), vec![(0, 0, 0)]);
    }
}
