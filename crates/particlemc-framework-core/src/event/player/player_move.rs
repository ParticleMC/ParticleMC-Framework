//! 玩家移动事件。

use crate::component::Position;
use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, InstanceEvent, PlayerEvent};
use crate::prelude::{Entity, Message};
use particlemc_framework_ecs::scheduler::WorldId;

/// 玩家移动事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerMove {
    /// 玩家实体。
    pub player: Entity,
    /// 移动前位置。
    pub from: Position,
    /// 移动后位置。
    pub to: Position,
    /// 是否已取消。
    pub cancelled: bool,
    /// 实例世界 id。
    pub instance_id: Option<WorldId>,
}

impl Event for PlayerMove {}

impl EntityEvent for PlayerMove {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for PlayerMove {}

impl InstanceEvent for PlayerMove {
    fn instance_id(&self) -> Option<WorldId> {
        self.instance_id
    }
}

impl CancellableEvent for PlayerMove {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
