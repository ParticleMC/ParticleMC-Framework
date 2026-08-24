// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 玩家选择方块事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 玩家选择方块事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerPickBlock {
    pub player: Entity,
    pub cancelled: bool,
}

impl Event for PlayerPickBlock {}
impl EntityEvent for PlayerPickBlock {
    fn entity(&self) -> Entity {
        self.player
    }
}
impl PlayerEvent for PlayerPickBlock {}
impl CancellableEvent for PlayerPickBlock {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
