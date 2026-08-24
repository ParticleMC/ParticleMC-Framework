// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 背包打开事件。

use super::InventoryType;
use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 背包打开事件。
#[derive(Message, Debug, Clone)]
pub struct InventoryOpen {
    /// 玩家实体。
    pub player: Entity,
    /// 背包类型。
    pub inventory_type: InventoryType,
    /// 是否已取消。
    pub cancelled: bool,
}

impl Event for InventoryOpen {}

impl EntityEvent for InventoryOpen {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for InventoryOpen {}

impl CancellableEvent for InventoryOpen {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
