// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 玩家选择实体事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 玩家选择实体事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerPickEntity {
    /// 玩家实体。
    pub player: Entity,
    /// 被选择的实体。
    pub target: Entity,
    /// 是否已取消。
    pub cancelled: bool,
}

impl Event for PlayerPickEntity {}

impl EntityEvent for PlayerPickEntity {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for PlayerPickEntity {}

impl CancellableEvent for PlayerPickEntity {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
