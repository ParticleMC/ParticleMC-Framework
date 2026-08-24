// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 玩家加载完成事件。

use crate::event::r#trait::{EntityEvent, Event, InstanceEvent, PlayerEvent};
use crate::prelude::{Entity, Message};
use particlemc_framework_ecs::scheduler::WorldId;

/// 玩家加载完成事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerLoaded {
    /// 玩家实体。
    pub player: Entity,
    /// 实例世界 id。
    pub instance_id: WorldId,
}

impl Event for PlayerLoaded {}

impl EntityEvent for PlayerLoaded {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for PlayerLoaded {}

impl InstanceEvent for PlayerLoaded {
    fn instance_id(&self) -> Option<WorldId> {
        Some(self.instance_id)
    }
}
