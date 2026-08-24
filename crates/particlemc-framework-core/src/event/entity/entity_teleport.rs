//! 实体传送事件。

use crate::component::Position;
use crate::event::r#trait::{CancellableEvent, EntityEvent, Event};
use crate::prelude::{Entity, Message};

/// 实体传送事件。
#[derive(Message, Debug, Clone)]
pub struct EntityTeleport {
    /// 传送的实体。
    pub entity: Entity,
    /// 传送前位置。
    pub from: Position,
    /// 传送后位置。
    pub to: Position,
    /// 是否已取消。
    pub cancelled: bool,
}

impl Event for EntityTeleport {}

impl EntityEvent for EntityTeleport {
    fn entity(&self) -> Entity {
        self.entity
    }
}

impl CancellableEvent for EntityTeleport {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
