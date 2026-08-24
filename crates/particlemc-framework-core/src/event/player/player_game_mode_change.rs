// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 玩家游戏模式变更事件。

use crate::component::GameMode;
use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 玩家游戏模式变更事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerGameModeChange {
    /// 玩家实体。
    pub player: Entity,
    /// 原游戏模式。
    pub old_mode: GameMode,
    /// 新游戏模式。
    pub new_mode: GameMode,
    /// 是否已取消。
    pub cancelled: bool,
}

impl Event for PlayerGameModeChange {}

impl EntityEvent for PlayerGameModeChange {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for PlayerGameModeChange {}

impl CancellableEvent for PlayerGameModeChange {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
