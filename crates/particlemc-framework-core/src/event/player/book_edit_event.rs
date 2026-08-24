//! 书本编辑事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 书本编辑事件（serverbound `EditBook`）。
#[derive(Message, Debug, Clone)]
pub struct BookEditEvent {
    /// 编辑书本的玩家实体。
    pub player: Entity,
    /// 使用的副手。
    pub hand: i32,
    /// 编辑后的页面内容（每页对应一个字符串）。
    pub pages: Vec<String>,
    /// 是否已取消。
    pub cancelled: bool,
}

impl Event for BookEditEvent {}

impl EntityEvent for BookEditEvent {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for BookEditEvent {}

impl CancellableEvent for BookEditEvent {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
