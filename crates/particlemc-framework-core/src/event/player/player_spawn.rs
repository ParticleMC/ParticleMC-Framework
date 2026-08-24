//! 玩家生成事件。

use crate::component::Position;
use crate::event::r#trait::{EntityEvent, Event, InstanceEvent, PlayerEvent};
use crate::prelude::{Entity, Message};
use particlemc_framework_ecs::scheduler::WorldId;

/// 玩家生成事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerSpawn {
    /// 玩家实体。
    pub player: Entity,
    /// 生成实例世界 id。
    pub instance_id: WorldId,
    /// 生成位置。
    pub position: Position,
}

impl Event for PlayerSpawn {}

impl EntityEvent for PlayerSpawn {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for PlayerSpawn {}

impl InstanceEvent for PlayerSpawn {
    fn instance_id(&self) -> Option<WorldId> {
        Some(self.instance_id)
    }
}
