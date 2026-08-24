// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 实体 tick 事件。

use crate::event::r#trait::{EntityEvent, Event};
use crate::prelude::{Entity, Message};

/// 实体 tick 事件。
#[derive(Message, Debug, Clone)]
pub struct EntityTick {
    /// 被 tick 的实体。
    pub entity: Entity,
}

impl Event for EntityTick {}

impl EntityEvent for EntityTick {
    fn entity(&self) -> Entity {
        self.entity
    }
}
