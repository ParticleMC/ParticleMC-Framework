// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 实体受伤事件。

use crate::component::DamageSource;
use crate::event::r#trait::{CancellableEvent, EntityEvent, Event};
use crate::prelude::{Entity, Message};
use crate::resource::DamageType;

/// 实体受伤事件。
#[derive(Message, Debug, Clone)]
pub struct EntityDamage {
    /// 受伤实体。
    pub entity: Entity,
    /// 伤害量。
    pub amount: f32,
    /// 伤害来源。
    pub source: DamageSource,
    /// 伤害类型。
    pub damage_type: Option<DamageType>,
    /// 是否已取消。
    pub cancelled: bool,
}

impl Event for EntityDamage {}

impl EntityEvent for EntityDamage {
    fn entity(&self) -> Entity {
        self.entity
    }
}

impl CancellableEvent for EntityDamage {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
