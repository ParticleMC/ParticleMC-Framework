//! 玩家手部动画事件。

use crate::event::r#trait::{EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 玩家手部动画事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerHandAnimation {
    pub player: Entity,
}

impl Event for PlayerHandAnimation {}
impl EntityEvent for PlayerHandAnimation {
    fn entity(&self) -> Entity {
        self.player
    }
}
impl PlayerEvent for PlayerHandAnimation {}
