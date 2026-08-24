// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! tick 管线第四步：实体 AI 计算（T6 升级）。
//!
//! 对挂载 `Living.ai = Some(group)` 且带 `Position` / `EntityCreature` /
//! `Velocity` 的实体：
//!
//! 1. `TargetSelector::find_target` 依据世界查找目标实体；
//! 2. `GoalSelector` 注入上下文并推进 goal（移动类 goal 暴露期望导航目标）；
//! 3. 把运行中 goal 的导航目标写入 `EntityCreature.navigation_target`
//!    （T4 数值字段契约保持，不引入 Navigator 字段）；
//! 4. 存在导航目标时以 [`GroundNodeFollower`] 计算速度并写入 `Velocity`
//!    （无 Navigator 的简化直移路径）。
//!
//! 签名说明（T11 迁移）：本系统采用排他系统 `fn(&mut World)` 单参数——
//! 共享读取经 `World::get`，既有组件写入经 `World::get_mut`，实体枚举经
//! `World::entities`（`archetype` 实体表遍历的等价）。这同时满足
//! `Target::find(&self, &World, ...)` 的签名契约。

use crate::prelude::{Entity, World};

use crate::component::{EntityCreature, Living, Position, Velocity};
use crate::entity::ai::GoalContext;
use crate::entity::pathfinding::{GroundNodeFollower, NodeFollower};

/// 地面跟随器（系统级共享，计算水平面速度）。
const GROUND_FOLLOWER: GroundNodeFollower = GroundNodeFollower;

/// 更新实体 AI 速度与导航目标。
pub fn entity_ai(world: &mut World) {
    // 1. 收集候选：枚举全部实体，筛出挂载 `Living.ai` 且带位置/生物组件的实体。
    let mut candidates: Vec<(Entity, [f64; 3])> = Vec::new();
    for entity in world.entities() {
        let Some(living) = world.get::<Living>(entity) else {
            continue;
        };
        if living.ai.is_none() {
            continue;
        }
        if world.get::<EntityCreature>(entity).is_none() {
            continue;
        }
        let Some(pos) = world.get::<Position>(entity) else {
            continue;
        };
        candidates.push((entity, [pos.x, pos.y, pos.z]));
    }

    // 2. 处理每个候选：目标查找 → goal tick → 写导航目标 → 计算速度。
    for (entity, self_position) in candidates {
        let target = world
            .get::<Living>(entity)
            .and_then(|l| l.ai.as_ref())
            .and_then(|group| group.targets.find_target(world, entity));
        let target_position = target
            .and_then(|t| world.get::<Position>(t))
            .map(|p| [p.x, p.y, p.z]);

        // goal tick（需要可变访问 Living；作用域内完成读取，随后再写其它组件）。
        let (nav, speed) = {
            let living = match world.get_mut::<Living>(entity) {
                Some(living) => living,
                None => continue,
            };
            let Some(group) = living.ai.as_mut() else {
                continue;
            };
            group.goals.update_context(&GoalContext {
                self_position,
                target,
                target_position,
            });
            group.goals.tick();
            (
                group.goals.navigation_target(),
                group.goals.movement_speed(),
            )
        };

        // 3. 写回 navigation_target（T4 数值字段契约）。
        if let Some(creature) = world.get_mut::<EntityCreature>(entity) {
            creature.navigation_target = nav;
        }

        // 4. 有导航目标时经地面跟随器计算速度。
        if let Some(target_point) = nav {
            let [vx, vy, vz] = GROUND_FOLLOWER.next_velocity(self_position, target_point, speed);
            if let Some(vel) = world.get_mut::<Velocity>(entity) {
                vel.x = vx;
                vel.y = vy;
                vel.z = vz;
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::app::App;

    use crate::entity::ai::{ClosestEntityTarget, EntityAIGroup, FollowTargetGoal};

    #[test]
    fn ai_entity_gains_velocity_toward_target() {
        let mut app = App::new();
        app.add_systems(entity_ai);

        // 目标实体。
        let target = app
            .world_mut()
            .spawn_bundle(Position::new(10.0, 64.0, 0.0))
            .id();

        // AI 实体：目标查找（最近实体）+ 跟随 goal。
        let mut group = EntityAIGroup::default();
        group
            .targets
            .add_target(10, ClosestEntityTarget::new(50.0, None));
        group.goals.add_goal(10, FollowTargetGoal::new(target, 2.0));
        let ai = app
            .world_mut()
            .spawn_bundle((
                Position::new(0.0, 64.0, 0.0),
                Velocity::zero(),
                Living { ai: Some(group) },
                EntityCreature::new(),
            ))
            .id();

        app.update();

        let world = app.world_mut();
        let q = world.query::<(&Velocity, &EntityCreature), ()>();
        let (vel, creature) = q.get(ai).expect("AI 实体应存在");
        // navigation_target 应写回，速度应指向目标（x 正方向）。
        assert!(creature.navigation_target.is_some(), "应写回导航目标");
        assert!(vel.x > 0.0, "x 轴速度应指向目标，实际 vx = {}", vel.x);
        assert!(vel.z.abs() < 1e-9, "z 轴速度应为零，实际 vz = {}", vel.z);
    }

    #[test]
    fn ai_entity_without_target_keeps_velocity_zero() {
        let mut app = App::new();
        app.add_systems(entity_ai);

        // AI 实体无任何目标实体（空世界仅自身）。
        let mut group = EntityAIGroup::default();
        group
            .targets
            .add_target(10, ClosestEntityTarget::new(50.0, None));
        group.goals.add_goal(10, FollowTargetGoal::new_unset(2.0));
        let ai = app
            .world_mut()
            .spawn_bundle((
                Position::new(0.0, 64.0, 0.0),
                Velocity::zero(),
                Living { ai: Some(group) },
                EntityCreature::new(),
            ))
            .id();

        app.update();

        let world = app.world_mut();
        let q = world.query::<(&Velocity, &EntityCreature), ()>();
        let (vel, creature) = q.get(ai).expect("AI 实体应存在");
        assert!(creature.navigation_target.is_none());
        assert_eq!(vel.x, 0.0);
        assert_eq!(vel.y, 0.0);
        assert_eq!(vel.z, 0.0);
    }
}
