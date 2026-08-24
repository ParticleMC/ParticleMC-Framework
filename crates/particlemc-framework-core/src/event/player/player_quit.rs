//! 玩家离开事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, InstanceEvent, PlayerEvent};
use crate::prelude::{Entity, Message};
use particlemc_framework_ecs::scheduler::WorldId;

/// 玩家离开事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerQuit {
    /// 玩家实体。
    pub player: Entity,
    /// 玩家用户名。
    pub username: String,
    /// 离开原因。
    pub reason: String,
    /// 是否已取消。
    pub cancelled: bool,
    /// 实例世界 id。
    pub instance_id: Option<WorldId>,
}

impl Event for PlayerQuit {}

impl EntityEvent for PlayerQuit {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for PlayerQuit {}

impl InstanceEvent for PlayerQuit {
    fn instance_id(&self) -> Option<WorldId> {
        self.instance_id
    }
}

impl CancellableEvent for PlayerQuit {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
