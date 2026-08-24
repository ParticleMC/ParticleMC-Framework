// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 实体攻击事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event};
use crate::prelude::{Entity, Message};

/// 实体攻击事件。
#[derive(Message, Debug, Clone)]
pub struct EntityAttack {
    /// 攻击者实体。
    pub entity: Entity,
    /// 被攻击目标实体。
    pub target: Entity,
    /// 是否已取消。
    pub cancelled: bool,
}

impl Event for EntityAttack {}

impl EntityEvent for EntityAttack {
    fn entity(&self) -> Entity {
        self.entity
    }
}

impl CancellableEvent for EntityAttack {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
