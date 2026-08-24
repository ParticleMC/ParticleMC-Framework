// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 背包关闭事件。

use super::InventoryType;
use crate::event::r#trait::{EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 背包关闭事件。
#[derive(Message, Debug, Clone)]
pub struct InventoryClose {
    /// 玩家实体。
    pub player: Entity,
    /// 背包类型。
    pub inventory_type: InventoryType,
}

impl Event for InventoryClose {}

impl EntityEvent for InventoryClose {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for InventoryClose {}
