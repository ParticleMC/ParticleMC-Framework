//! 实体间碰撞检测：沿速度矢量扫掠，检测与附近实体的碰撞。
//!
//! 对齐 Java [`net.minestom.server.collision.EntityCollision`] 的框架层抽象。
//! 本模块通过 `nearby` 钩子解耦实体管理器，使框架不依赖具体 ECS 查询。
//!
//! 变更标识符：`complete-collision-physics`（WS6）。
//! 见 `.specs/complete-collision-physics/spec.md`。

use crate::physics::{Aabb, SweepResult};

/// 实体碰撞结果。
#[derive(Debug)]
pub struct EntityCollisionResult {
    /// 碰撞点（世界坐标 `[x, y, z]`）。
    pub collision_point: [f64; 3],
    /// 碰撞实体 ID。
    pub entity_id: u64,
    /// 碰撞法线方向（从碰撞实体指向本实体，归一化前）。
    pub direction: [f64; 3],
    /// 碰撞比例（`[0, 1]`，越小越早碰撞）。
    pub percentage: f64,
}

/// 附近实体查询结果项。
pub type NearbyEntity<'a> = (u64, [f64; 3], &'a Aabb);

/// 附近实体查询闭包：给定搜索中心与半径，返回该范围内所有实体。
///
/// 框架层通过此钩子注入实体管理器查询，避免框架耦合具体 ECS。
/// 闭包须返回 `'static` 引用（测试中用 `Box::leak` 或静态数据）。
pub type NearbyFn = dyn Fn([f64; 3], f64) -> Vec<NearbyEntity<'static>>;

/// 实体过滤闭包：返回 `true` 表示排除该实体（如自身）。
pub type EntityFilter = dyn Fn(u64) -> bool;

/// 检测沿 `velocity` 移动的实体 `src_id` 是否与附近实体碰撞。
///
/// - `position`：本实体脚底中心世界坐标。
/// - `bounding_box`：本实体当前碰撞盒。
/// - `velocity`：本 tick 速度向量（方块/tick）。
/// - `extend_radius`：搜索半径；建议 ≥ 最大实体尺寸对角线 + `|velocity|`。
/// - `filter`：排除特定实体（如自身）。
/// - `nearby`：附近实体查询钩子。
///
/// 返回按碰撞比例升序排列的碰撞结果列表（最早碰撞在前）。
pub fn check_entity_collision(
    _src_id: u64,
    position: [f64; 3],
    bounding_box: &Aabb,
    velocity: [f64; 3],
    extend_radius: f64,
    filter: &EntityFilter,
    nearby: &NearbyFn,
) -> Vec<EntityCollisionResult> {
    let mut results = Vec::new();
    let mut best_res = SweepResult::NO_COLLISION;

    // 搜索半径：以速度长度扩展
    let search_radius = extend_radius + velocity.iter().map(|&v| v.abs()).sum::<f64>().sqrt();

    for (entity_id, ent_pos, ent_aabb) in nearby(position, search_radius) {
        if filter(entity_id) {
            continue;
        }

        // 先做静态相交检测（重叠时无需扫掠）
        if bounding_box.intersects(ent_aabb) {
            results.push(EntityCollisionResult {
                collision_point: ent_pos,
                entity_id,
                direction: [0.0, 0.0, 0.0],
                percentage: 0.0,
            });
            continue;
        }

        // 扫掠检测
        if let Some(sweep) = bounding_box.sweep_intersection(velocity, ent_aabb)
            && sweep.res < best_res.res
        {
            best_res = sweep;
            // 清理旧结果中更晚的碰撞
            results.retain(|r| r.percentage < sweep.res);
            results.push(EntityCollisionResult {
                collision_point: [sweep.collided_x, sweep.collided_y, sweep.collided_z],
                entity_id,
                direction: [sweep.normal_x, sweep.normal_y, sweep.normal_z],
                percentage: sweep.res,
            });
        }
    }

    // 按碰撞比例升序排序（最早碰撞在前）
    results.sort_by(|a, b| {
        a.percentage
            .partial_cmp(&b.percentage)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results
}

/// 计算实体碰撞盒的最大可能对角线长度，用于确定搜索半径下界。
#[must_use]
pub fn max_entity_diagonal(width: f64, height: f64) -> f64 {
    // 对角线 = sqrt(width² + height² + width²)（x/z 对称）
    (width * width + height * height + width * width).sqrt()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn make_nearby_empty(_center: [f64; 3], _radius: f64) -> Vec<NearbyEntity<'static>> {
        Vec::new()
    }

    fn make_filter_always_true(_id: u64) -> bool {
        true
    }

    #[test]
    fn no_nearby_entities_returns_empty() {
        let aabb = Aabb::from_pos_size([0.0, 0.0, 0.0], [0.6, 1.8, 0.6]);
        let results = check_entity_collision(
            1,
            [0.0, 0.0, 0.0],
            &aabb,
            [1.0, 0.0, 0.0],
            2.0,
            &make_filter_always_true,
            &make_nearby_empty,
        );
        assert!(results.is_empty());
    }

    #[test]
    fn static_overlap_detected() {
        let aabb = Aabb::from_pos_size([0.0, 0.0, 0.0], [0.6, 1.8, 0.6]);
        // 另一个实体与本实体静态重叠
        let other_aabb = Aabb::from_pos_size([0.2, 0.0, 0.0], [0.8, 1.8, 0.6]);
        let nearby = move |_center: [f64; 3], _radius: f64| {
            let leaked: *const Aabb = Box::leak(Box::new(other_aabb));
            vec![(2u64, [0.5, 0.9, 0.3], unsafe { &*leaked })]
        };
        let filter = |id: u64| id == 1;
        let results = check_entity_collision(
            1,
            [0.0, 0.0, 0.0],
            &aabb,
            [0.0, 0.0, 0.0],
            2.0,
            &filter,
            &nearby,
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entity_id, 2);
        assert_eq!(results[0].percentage, 0.0);
    }

    #[test]
    fn sweep_collision_found() {
        let aabb = Aabb::from_pos_size([0.0, 0.0, 0.0], [0.6, 1.8, 0.6]);
        // 实体 B 在 x=1.5..2.1，A 向右移动 1.0 → 碰撞发生在 t=0.9（a.max=0.6 到达 b.min=1.5）
        let other_aabb = Aabb::from_pos_size([1.5, 0.0, 0.0], [2.1, 1.8, 0.6]);
        let nearby = move |_center: [f64; 3], _radius: f64| {
            let leaked: *const Aabb = Box::leak(Box::new(other_aabb));
            vec![(2u64, [1.8, 0.9, 0.3], unsafe { &*leaked })]
        };
        let filter = |id: u64| id == 1;
        let results = check_entity_collision(
            1,
            [0.0, 0.0, 0.0],
            &aabb,
            [1.0, 0.0, 0.0],
            3.0,
            &filter,
            &nearby,
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entity_id, 2);
        assert!(
            (results[0].percentage - 0.9).abs() < 1e-6,
            "percentage = {}",
            results[0].percentage
        );
    }

    #[test]
    fn filter_excludes_self() {
        let aabb = Aabb::from_pos_size([0.0, 0.0, 0.0], [0.6, 1.8, 0.6]);
        let nearby = move |_center: [f64; 3], _radius: f64| {
            let leaked: *const Aabb = Box::leak(Box::new(aabb));
            vec![(1u64, [0.0, 0.0, 0.0], unsafe { &*leaked })]
        };
        // 过滤掉 ID=1（自身）
        let filter = |id: u64| id == 1;
        let results = check_entity_collision(
            1,
            [0.0, 0.0, 0.0],
            &aabb,
            [1.0, 0.0, 0.0],
            2.0,
            &filter,
            &nearby,
        );
        assert!(results.is_empty());
    }

    #[test]
    fn max_entity_diagonal_correct() {
        // 0.6 宽 × 1.8 高：对角线 = sqrt(0.6² + 1.8² + 0.6²) = sqrt(0.36 + 3.24 + 0.36) = sqrt(3.96) ≈ 1.99
        let diag = max_entity_diagonal(0.6, 1.8);
        assert!((diag - 1.98997487421324).abs() < 1e-6);
    }
}
