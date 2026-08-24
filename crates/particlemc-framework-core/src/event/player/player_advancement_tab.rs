// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 玩家进度标签事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 玩家进度标签事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerAdvancementTab {
    pub player: Entity,
    pub opened: bool,
    pub cancelled: bool,
}

impl Event for PlayerAdvancementTab {}
impl EntityEvent for PlayerAdvancementTab {
    fn entity(&self) -> Entity {
        self.player
    }
}
impl PlayerEvent for PlayerAdvancementTab {}
impl CancellableEvent for PlayerAdvancementTab {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
