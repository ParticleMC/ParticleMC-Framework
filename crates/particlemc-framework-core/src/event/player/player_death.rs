// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 玩家死亡事件。

use crate::event::r#trait::{EntityEvent, Event, InstanceEvent, PlayerEvent};
use crate::prelude::{Entity, Message};
use particlemc_framework_ecs::scheduler::WorldId;

/// 玩家死亡事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerDeath {
    /// 玩家实体。
    pub player: Entity,
    /// 死亡消息。
    pub death_message: String,
    /// 实例世界 id。
    pub instance_id: Option<WorldId>,
}

impl Event for PlayerDeath {}

impl EntityEvent for PlayerDeath {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for PlayerDeath {}

impl InstanceEvent for PlayerDeath {
    fn instance_id(&self) -> Option<WorldId> {
        self.instance_id
    }
}
