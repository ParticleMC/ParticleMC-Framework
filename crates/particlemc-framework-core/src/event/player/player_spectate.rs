// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 玩家旁观事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 玩家旁观事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerSpectate {
    pub player: Entity,
    pub target: Entity,
    pub cancelled: bool,
}

impl Event for PlayerSpectate {}
impl EntityEvent for PlayerSpectate {
    fn entity(&self) -> Entity {
        self.player
    }
}
impl PlayerEvent for PlayerSpectate {}
impl CancellableEvent for PlayerSpectate {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
