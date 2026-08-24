// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 玩家数据包入站事件。

use crate::event::r#trait::{EntityEvent, Event, InstanceEvent, PlayerEvent};
use crate::prelude::{Entity, Message};
use particlemc_framework_ecs::scheduler::WorldId;

/// 玩家数据包事件（入站）。
#[derive(Message, Debug, Clone)]
pub struct PlayerPacket {
    pub player: Entity,
    pub packet_id: i32,
    pub instance_id: Option<WorldId>,
}

impl Event for PlayerPacket {}
impl EntityEvent for PlayerPacket {
    fn entity(&self) -> Entity {
        self.player
    }
}
impl PlayerEvent for PlayerPacket {}
impl InstanceEvent for PlayerPacket {
    fn instance_id(&self) -> Option<WorldId> {
        self.instance_id
    }
}
