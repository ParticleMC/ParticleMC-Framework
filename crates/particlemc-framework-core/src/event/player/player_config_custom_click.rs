//! 玩家自定义配置点击事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 玩家自定义配置点击事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerConfigCustomClick {
    pub player: Entity,
    pub button: u8,
    pub slot: u8,
    pub cancelled: bool,
}

impl Event for PlayerConfigCustomClick {}
impl EntityEvent for PlayerConfigCustomClick {
    fn entity(&self) -> Entity {
        self.player
    }
}
impl PlayerEvent for PlayerConfigCustomClick {}
impl CancellableEvent for PlayerConfigCustomClick {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
