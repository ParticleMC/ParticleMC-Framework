//! 玩家编辑告示板事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 玩家编辑告示板事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerEditSign {
    /// 玩家实体。
    pub player: Entity,
    /// 告示板正面。
    pub front_text: bool,
    /// 第 1 行文本。
    pub line_1: String,
    /// 第 2 行文本。
    pub line_2: String,
    /// 第 3 行文本。
    pub line_3: String,
    /// 第 4 行文本。
    pub line_4: String,
    /// 是否已取消。
    pub cancelled: bool,
}

impl Event for PlayerEditSign {}

impl EntityEvent for PlayerEditSign {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for PlayerEditSign {}

impl CancellableEvent for PlayerEditSign {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
