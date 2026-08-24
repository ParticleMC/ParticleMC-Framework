// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 玩家取消使用物品事件。

use crate::event::item::Hand;
use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 玩家取消使用物品事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerCancelItemUse {
    /// 玩家实体。
    pub player: Entity,
    /// 使用的手。
    pub hand: Hand,
    /// 是否已取消。
    pub cancelled: bool,
}

impl Event for PlayerCancelItemUse {}

impl EntityEvent for PlayerCancelItemUse {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for PlayerCancelItemUse {}

impl CancellableEvent for PlayerCancelItemUse {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
