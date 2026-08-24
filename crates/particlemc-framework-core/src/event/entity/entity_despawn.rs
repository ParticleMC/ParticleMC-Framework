// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 实体消失事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event};
use crate::prelude::{Entity, Message};

/// 实体消失原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DespawnReason {
    /// 实体死亡。
    Death,
    /// 玩家断开连接。
    PlayerQuit,
    /// 主动取消生成。
    Despawn,
}

/// 实体消失事件。
#[derive(Message, Debug, Clone)]
pub struct EntityDespawn {
    /// 消失的实体。
    pub entity: Entity,
    /// 消失原因。
    pub reason: DespawnReason,
    /// 是否已取消。
    pub cancelled: bool,
}

impl Event for EntityDespawn {}

impl EntityEvent for EntityDespawn {
    fn entity(&self) -> Entity {
        self.entity
    }
}

impl CancellableEvent for EntityDespawn {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
