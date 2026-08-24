// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 玩家开始使用物品事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 使用手的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hand {
    /// 主手。
    MainHand,
    /// 副手。
    OffHand,
}

/// 玩家开始使用物品事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerBeginItemUse {
    /// 玩家实体。
    pub player: Entity,
    /// 使用的手。
    pub hand: Hand,
    /// 是否已取消。
    pub cancelled: bool,
}

impl Event for PlayerBeginItemUse {}

impl EntityEvent for PlayerBeginItemUse {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for PlayerBeginItemUse {}

impl CancellableEvent for PlayerBeginItemUse {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
