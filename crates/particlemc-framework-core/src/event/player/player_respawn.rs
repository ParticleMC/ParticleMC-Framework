//! 玩家重生事件。

use crate::component::Position;
use crate::event::r#trait::{EntityEvent, Event, InstanceEvent, PlayerEvent};
use crate::prelude::{Entity, Message};
use particlemc_framework_ecs::scheduler::WorldId;

/// 玩家重生事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerRespawn {
    /// 玩家实体。
    pub player: Entity,
    /// 重生实例世界 id。
    pub instance_id: WorldId,
    /// 重生位置。
    pub position: Position,
}

impl Event for PlayerRespawn {}

impl EntityEvent for PlayerRespawn {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for PlayerRespawn {}

impl InstanceEvent for PlayerRespawn {
    fn instance_id(&self) -> Option<WorldId> {
        Some(self.instance_id)
    }
}
