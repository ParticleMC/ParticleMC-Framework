// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 实体速度事件。

use crate::component::Velocity;
use crate::event::r#trait::{EntityEvent, Event};
use crate::prelude::{Entity, Message};

/// 实体速度事件。
#[derive(Message, Debug, Clone)]
pub struct EntityVelocity {
    /// 实体。
    pub entity: Entity,
    /// 速度。
    pub velocity: Velocity,
}

impl Event for EntityVelocity {}

impl EntityEvent for EntityVelocity {
    fn entity(&self) -> Entity {
        self.entity
    }
}
