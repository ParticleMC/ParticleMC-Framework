// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 玩家命令事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 玩家命令事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerCommand {
    /// 玩家实体。
    pub player: Entity,
    /// 命令文本。
    pub command: String,
    /// 是否已取消。
    pub cancelled: bool,
}

impl Event for PlayerCommand {}

impl EntityEvent for PlayerCommand {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for PlayerCommand {}

impl CancellableEvent for PlayerCommand {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
