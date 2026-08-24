//! 活体实体组件（T4 实体类层级：以组件组合替代 Java `LivingEntity` 继承）。
//!
//! [`Living`] 标记「有生命值、可受伤」的实体，携带 AI 组（[`EntityAIGroup`]，
//! T6 由 `entity::ai` 提供，取代本文件 T4 空壳）；受伤结算委托
//! [`Health::damage`]，经 [`HurtResult`] 区分存活 / 死亡分支。`Player` 等
//! 具体实体在生成时作为独立组件挂载 `Living`（不在 `Player` 结构体内聚合），
//! 受伤可经挂载 `Living` 后调用 [`Living::hurt`]，或直接操作 `Health`。
//!
//! 变更标识符：`complete-missing-subsystems`（T4 / T6）。

use crate::prelude::Component;

use crate::component::{Damage, Health};

pub use crate::entity::ai::EntityAIGroup;

/// 活体实体组件。
#[derive(Component, Default)]
#[component(storage = "sparse")]
pub struct Living {
    /// AI 组（T6 起为真实 [`EntityAIGroup`]，未配置 AI 时恒为 `None`）。
    pub ai: Option<EntityAIGroup>,
}

/// 受伤结算结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HurtResult {
    /// 受伤后仍存活。
    Alive,
    /// 受伤导致死亡。
    Died,
}

impl Living {
    /// 对实体造成一次伤害并结算。
    ///
    /// 委托 [`Health::damage`] 扣血（`amount` 非正时被忽略，实体保持存活），
    /// 依据受伤后生命值返回 [`HurtResult::Alive`] 或 [`HurtResult::Died`]。
    ///
    /// T6 填充 AI 后 `hurt` 将依据 [`Living::ai`] 应用受伤修正（如伤害倍率），
    /// 故当前需要 `&mut self` 以支持未来 AI 修正。
    pub fn hurt(&mut self, health: &mut Health, damage: &Damage) -> HurtResult {
        health.damage(damage.amount);
        if health.is_alive() {
            HurtResult::Alive
        } else {
            HurtResult::Died
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::prelude::World;

    #[test]
    fn hurt_alive_when_health_remains() {
        let mut living = Living::default();
        let mut health = Health::new(20.0, 20.0);
        assert_eq!(
            living.hurt(&mut health, &Damage::new(5.0)),
            HurtResult::Alive
        );
        assert_eq!(health.current, 15.0);
    }

    #[test]
    fn hurt_died_when_health_reaches_zero() {
        let mut living = Living::default();
        let mut health = Health::new(5.0, 20.0);
        assert_eq!(
            living.hurt(&mut health, &Damage::new(10.0)),
            HurtResult::Died
        );
        assert_eq!(health.current, 0.0);
    }

    #[test]
    fn hurt_ignores_non_positive_damage_and_stays_alive() {
        let mut living = Living::default();
        let mut health = Health::new(20.0, 20.0);
        assert_eq!(
            living.hurt(&mut health, &Damage::new(0.0)),
            HurtResult::Alive
        );
        assert_eq!(
            living.hurt(&mut health, &Damage::new(-3.0)),
            HurtResult::Alive
        );
        assert_eq!(health.current, 20.0);
    }

    #[test]
    fn living_defaults_to_no_ai() {
        let living = Living::default();
        assert!(living.ai.is_none());
    }

    #[test]
    fn living_and_ai_group_can_be_spawned() {
        let mut world = World::new();
        let entity = world.spawn_bundle(Living::default()).id();
        assert!(world.get::<Living>(entity).is_some());

        let ai_entity = world.spawn_bundle(EntityAIGroup::default()).id();
        assert!(world.get::<EntityAIGroup>(ai_entity).is_some());
    }
}
