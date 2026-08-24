// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 拾取物品事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 拾取物品事件。
#[derive(Message, Debug, Clone)]
pub struct PickupItem {
    /// 玩家实体。
    pub player: Entity,
    /// 被拾取的物品实体。
    pub item: Entity,
    /// 拾取距离。
    pub distance: f64,
    /// 是否已取消。
    pub cancelled: bool,
}

impl Event for PickupItem {}

impl EntityEvent for PickupItem {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for PickupItem {}

impl CancellableEvent for PickupItem {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
