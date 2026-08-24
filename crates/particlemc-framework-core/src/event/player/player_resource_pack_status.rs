//! 玩家资源包状态事件。

use crate::event::r#trait::{EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 资源包状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourcePackStatus {
    Success,
    Declined,
    Failed,
    Downloaded,
    Accepted,
}

/// 玩家资源包状态事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerResourcePackStatus {
    pub player: Entity,
    pub status: ResourcePackStatus,
}

impl Event for PlayerResourcePackStatus {}
impl EntityEvent for PlayerResourcePackStatus {
    fn entity(&self) -> Entity {
        self.player
    }
}
impl PlayerEvent for PlayerResourcePackStatus {}
