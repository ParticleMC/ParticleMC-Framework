//! 玩家离开床事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 玩家离开床事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerLeaveBed {
    pub player: Entity,
    pub cancelled: bool,
}

impl Event for PlayerLeaveBed {}
impl EntityEvent for PlayerLeaveBed {
    fn entity(&self) -> Entity {
        self.player
    }
}
impl PlayerEvent for PlayerLeaveBed {}
impl CancellableEvent for PlayerLeaveBed {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
