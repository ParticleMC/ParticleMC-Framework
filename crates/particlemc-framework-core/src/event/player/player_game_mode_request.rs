//! 玩家游戏模式请求事件。

use crate::component::GameMode;
use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 玩家游戏模式请求事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerGameModeRequest {
    /// 玩家实体。
    pub player: Entity,
    /// 请求的游戏模式。
    pub requested_mode: GameMode,
    /// 是否已取消。
    pub cancelled: bool,
}

impl Event for PlayerGameModeRequest {}

impl EntityEvent for PlayerGameModeRequest {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for PlayerGameModeRequest {}

impl CancellableEvent for PlayerGameModeRequest {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
