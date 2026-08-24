//! 弹射物实体组件（T4 实体类层级：以组件组合替代 Java `EntityProjectile` 继承）。
//!
//! [`EntityProjectile`] 记录射手、伤害量与速度，并承载命中回调。本任务仅定义
//! 数据与构造入口；实际命中检测（对目标实体结算伤害、触发 [`on_hit`]）由后续
//! tick 系统消费。
//!
//! 变更标识符：`complete-missing-subsystems`（T4）。

use crate::prelude::{Component, Entity, World};

/// 弹射物命中回调：在 `&mut World` 中处理命中逻辑，参数为被命中目标实体。
///
/// 类型别名仅为规避 `clippy::type_complexity`；其展开式与任务规格的
/// `Option<Box<dyn FnMut(&mut World, Entity) + Send + Sync>>` 一致。
pub type ProjectileHitFn = Box<dyn FnMut(&mut World, Entity) + Send + Sync>;

/// 弹射物实体组件。
#[derive(Default, Component)]
#[component(storage = "sparse")]
pub struct EntityProjectile {
    /// 射手实体（`None` 表示无射手，如环境生成的弹射物）。
    pub shooter: Option<Entity>,
    /// 命中目标时造成的伤害量。
    pub damage: f32,
    /// 当前速度（方块/秒，三轴）。
    pub velocity: [f64; 3],
    /// 命中回调（`None` 表示无回调）。
    pub on_hit: Option<ProjectileHitFn>,
}

impl EntityProjectile {
    /// 以射手、伤害量与速度构造弹射物（无命中回调，需后挂）。
    pub fn new(shooter: Option<Entity>, damage: f32, velocity: [f64; 3]) -> Self {
        Self {
            shooter,
            damage,
            velocity,
            on_hit: None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::prelude::World;

    #[test]
    fn new_holds_shooter_damage_and_velocity() {
        let entity = world_spawn_probe();
        let projectile = EntityProjectile::new(Some(entity), 6.0, [1.5, 2.0, -3.0]);
        assert_eq!(projectile.shooter, Some(entity));
        assert_eq!(projectile.damage, 6.0);
        assert_eq!(projectile.velocity, [1.5, 2.0, -3.0]);
        assert!(projectile.on_hit.is_none());
    }

    #[test]
    fn new_without_shooter_keeps_none() {
        let projectile = EntityProjectile::new(None, 4.0, [0.0, 0.0, 0.0]);
        assert_eq!(projectile.shooter, None);
    }

    #[test]
    fn projectile_component_can_be_spawned() {
        let mut world = World::new();
        let entity = world
            .spawn_bundle(EntityProjectile::new(None, 5.0, [0.0, 0.0, 0.0]))
            .id();
        assert!(world.get::<EntityProjectile>(entity).is_some());
    }

    #[test]
    fn projectile_can_spawn_with_on_hit_callback() {
        let mut world = World::new();
        let projectile = EntityProjectile {
            on_hit: Some(Box::new(|_: &mut World, _: Entity| {})),
            ..EntityProjectile::new(None, 3.0, [0.0, 0.0, 0.0])
        };
        let entity = world.spawn_bundle(projectile).id();
        let spawned = world.get::<EntityProjectile>(entity).unwrap();
        assert!(spawned.on_hit.is_some());
    }

    /// 构造一个真实存在的 旧 ECS 方案 `Entity` 供断言使用。
    fn world_spawn_probe() -> Entity {
        World::new().spawn_empty().id()
    }
}
