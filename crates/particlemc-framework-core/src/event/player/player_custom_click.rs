//! 玩家自定义点击事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 玩家自定义点击事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerCustomClick {
    pub player: Entity,
    pub button: u8,
    pub cancelled: bool,
}

impl Event for PlayerCustomClick {}
impl EntityEvent for PlayerCustomClick {
    fn entity(&self) -> Entity {
        self.player
    }
}
impl PlayerEvent for PlayerCustomClick {}
impl CancellableEvent for PlayerCustomClick {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
