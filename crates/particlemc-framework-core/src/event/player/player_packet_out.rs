//! 玩家数据包出站事件。

use crate::event::r#trait::{EntityEvent, Event, InstanceEvent, PlayerEvent};
use crate::prelude::{Entity, Message};
use particlemc_framework_ecs::scheduler::WorldId;

/// 玩家数据包事件（出站）。
#[derive(Message, Debug, Clone)]
pub struct PlayerPacketOut {
    pub player: Entity,
    pub packet_id: i32,
    pub instance_id: Option<WorldId>,
}

impl Event for PlayerPacketOut {}
impl EntityEvent for PlayerPacketOut {
    fn entity(&self) -> Entity {
        self.player
    }
}
impl PlayerEvent for PlayerPacketOut {}
impl InstanceEvent for PlayerPacketOut {
    fn instance_id(&self) -> Option<WorldId> {
        self.instance_id
    }
}
