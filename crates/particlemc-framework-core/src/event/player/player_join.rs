// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 玩家加入事件。

use crate::event::r#trait::{EntityEvent, Event, InstanceEvent, PlayerEvent};
use crate::prelude::{Entity, Message};
use particlemc_framework_ecs::scheduler::WorldId;

/// 玩家加入事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerJoin {
    /// 玩家实体。
    pub player: Entity,
    /// 玩家用户名。
    pub username: String,
    /// 实例世界 id（由外部设置）。
    pub instance_id: Option<WorldId>,
}

impl Event for PlayerJoin {}

impl EntityEvent for PlayerJoin {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for PlayerJoin {}

impl InstanceEvent for PlayerJoin {
    fn instance_id(&self) -> Option<WorldId> {
        self.instance_id
    }
}
