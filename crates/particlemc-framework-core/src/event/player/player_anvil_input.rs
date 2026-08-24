//! 玩家 Anvil 输入事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 玩家 Anvil 输入事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerAnvilInput {
    pub player: Entity,
    pub input: String,
    pub cost: u32,
    pub cancelled: bool,
}

impl Event for PlayerAnvilInput {}
impl EntityEvent for PlayerAnvilInput {
    fn entity(&self) -> Entity {
        self.player
    }
}
impl PlayerEvent for PlayerAnvilInput {}
impl CancellableEvent for PlayerAnvilInput {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
