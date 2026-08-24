//! 玩家调试订阅请求事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 玩家调试订阅请求事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerDebugSubscriptionsRequest {
    pub player: Entity,
    pub cancelled: bool,
}

impl Event for PlayerDebugSubscriptionsRequest {}
impl EntityEvent for PlayerDebugSubscriptionsRequest {
    fn entity(&self) -> Entity {
        self.player
    }
}
impl PlayerEvent for PlayerDebugSubscriptionsRequest {}
impl CancellableEvent for PlayerDebugSubscriptionsRequest {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
