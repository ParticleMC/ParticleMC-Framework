// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 玩家实体交互事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 交互动作类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractAction {
    /// 攻击。
    Attack,
    /// 交互。
    Interact,
    /// 互动物品。
    InteractAt,
}

/// 玩家实体交互事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerEntityInteract {
    /// 玩家实体。
    pub player: Entity,
    /// 目标实体。
    pub target: Entity,
    /// 交互动作。
    pub action: InteractAction,
    /// 是否已取消。
    pub cancelled: bool,
}

impl Event for PlayerEntityInteract {}

impl EntityEvent for PlayerEntityInteract {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for PlayerEntityInteract {}

impl CancellableEvent for PlayerEntityInteract {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
