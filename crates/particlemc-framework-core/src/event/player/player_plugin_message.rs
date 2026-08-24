// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 玩家插件消息事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 玩家插件消息事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerPluginMessage {
    pub player: Entity,
    pub channel: String,
    pub data: Vec<u8>,
    pub cancelled: bool,
}

impl Event for PlayerPluginMessage {}
impl EntityEvent for PlayerPluginMessage {
    fn entity(&self) -> Entity {
        self.player
    }
}
impl PlayerEvent for PlayerPluginMessage {}
impl CancellableEvent for PlayerPluginMessage {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
