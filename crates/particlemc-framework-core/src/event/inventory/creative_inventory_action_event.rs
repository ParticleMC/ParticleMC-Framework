// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 创造模式物品栏操作事件。

use crate::event::inventory::inventory_click::ClickAction;
use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 创造模式物品栏操作事件（creative inventory action）。
#[derive(Message, Debug, Clone)]
pub struct CreativeInventoryActionEvent {
    /// 操作的玩家实体。
    pub player: Entity,
    /// 被拖动的槽位。
    pub clicked_slot: u8,
    /// 目标槽位。
    pub target_slot: u8,
    /// 点击类型。
    pub click_action: ClickAction,
    /// 是否已取消。
    pub cancelled: bool,
}

impl Event for CreativeInventoryActionEvent {}

impl EntityEvent for CreativeInventoryActionEvent {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for CreativeInventoryActionEvent {}

impl CancellableEvent for CreativeInventoryActionEvent {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
