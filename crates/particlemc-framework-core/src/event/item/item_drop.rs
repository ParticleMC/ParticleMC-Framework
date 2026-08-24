// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 掉落物品事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 掉落物品事件。
#[derive(Message, Debug, Clone)]
pub struct ItemDrop {
    /// 玩家实体。
    pub player: Entity,
    /// 掉落的物品实体。
    pub item: Entity,
    /// 原所在槽位。
    pub slot: u8,
    /// 是否已取消。
    pub cancelled: bool,
}

impl Event for ItemDrop {}

impl EntityEvent for ItemDrop {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for ItemDrop {}

impl CancellableEvent for ItemDrop {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
