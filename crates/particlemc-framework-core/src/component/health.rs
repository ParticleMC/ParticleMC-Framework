// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 实体生命值组件（骨架层真实逻辑）。
//!
//! 维护 `current` / `max` 两个 `f32` 值，提供 `damage` / `heal` / `is_alive`
//! 最小逻辑。所有修改都会被钳制在 `[0, max]` 区间，负向治疗 / 伤害量被忽略，
//! 保证状态始终一致。`Health` 为 `Copy`。

use crate::prelude::Component;

/// 实体生命值。
#[derive(Default, Component, Debug, Clone, Copy, PartialEq)]
#[component(storage = "sparse")]
pub struct Health {
    /// 当前生命值。
    pub current: f32,
    /// 最大生命值。
    pub max: f32,
}

impl Health {
    /// 构造生命值，自动将 `current` 钳制到 `[0, max]`。
    ///
    /// 若 `max` 为负则视为 0，`current` 随之归零。
    pub fn new(current: f32, max: f32) -> Self {
        let maximum = max.max(0.0);
        let current = current.clamp(0.0, maximum);
        Self {
            current,
            max: maximum,
        }
    }

    /// 对实体造成伤害，生命值不会低于 0。
    ///
    /// 非正的伤害量将被忽略（不产生任何效果）。
    pub fn damage(&mut self, amount: f32) {
        if amount > 0.0 {
            self.current = (self.current - amount).max(0.0);
        }
    }

    /// 治疗实体，生命值不会超过 `max`。
    ///
    /// 非正的治疗量将被忽略。
    pub fn heal(&mut self, amount: f32) {
        if amount > 0.0 {
            self.current = (self.current + amount).min(self.max);
        }
    }

    /// 实体是否存活（当前生命值严格大于 0）。
    pub fn is_alive(&self) -> bool {
        self.current > 0.0
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn new_clamps_current_and_max() {
        let health = Health::new(25.0, 20.0);
        assert_eq!(health.current, 20.0);
        assert_eq!(health.max, 20.0);

        let negative = Health::new(5.0, -3.0);
        assert_eq!(negative.max, 0.0);
        assert_eq!(negative.current, 0.0);
    }

    #[test]
    fn damage_reduces_current_and_floors_at_zero() {
        let mut health = Health::new(20.0, 20.0);
        health.damage(5.0);
        assert_eq!(health.current, 15.0);

        health.damage(100.0);
        assert_eq!(health.current, 0.0);
        assert!(!health.is_alive());
    }

    #[test]
    fn damage_ignores_non_positive_amount() {
        let mut health = Health::new(20.0, 20.0);
        health.damage(0.0);
        health.damage(-5.0);
        assert_eq!(health.current, 20.0);
    }

    #[test]
    fn heal_increases_current_and_caps_at_max() {
        let mut health = Health::new(10.0, 20.0);
        health.heal(5.0);
        assert_eq!(health.current, 15.0);
        assert!(health.is_alive());

        health.heal(100.0);
        assert_eq!(health.current, 20.0);
    }

    #[test]
    fn heal_ignores_non_positive_amount() {
        let mut health = Health::new(10.0, 20.0);
        health.heal(0.0);
        health.heal(-5.0);
        assert_eq!(health.current, 10.0);
    }
}
