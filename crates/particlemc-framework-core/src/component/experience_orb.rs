// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 经验球实体组件（T4 实体类层级：以组件组合替代 Java `ExperienceOrb` 继承）。
//!
//! [`ExperienceOrb`] 记录经验值；拾取分配语义由后续 tick 系统消费。
//!
//! 变更标识符：`complete-missing-subsystems`（T4）。

use crate::prelude::Component;

/// 经验球实体组件。
#[derive(Default, Component, Clone, Copy, Debug, PartialEq, Eq)]
#[component(storage = "sparse")]
pub struct ExperienceOrb {
    /// 携带的经验值。
    pub experience: u32,
}

impl ExperienceOrb {
    /// 以经验值构造经验球。
    pub fn new(experience: u32) -> Self {
        Self { experience }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::prelude::World;

    #[test]
    fn new_holds_experience() {
        let orb = ExperienceOrb::new(7);
        assert_eq!(orb.experience, 7);
    }

    #[test]
    fn experience_orb_component_can_be_spawned() {
        let mut world = World::new();
        let entity = world.spawn_bundle(ExperienceOrb::new(3)).id();
        assert_eq!(world.get::<ExperienceOrb>(entity).unwrap().experience, 3);
    }
}
