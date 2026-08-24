//! 窗口按钮点击事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 窗口按钮点击事件（serverbound `ClickButton`）。
#[derive(Message, Debug, Clone)]
pub struct WindowButtonClickEvent {
    /// 点击按钮的玩家实体。
    pub player: Entity,
    /// 窗口交互 id（与 `open_window` 包中的 id 匹配）。
    pub window_id: u8,
    /// 被点击的按钮 id。
    pub button_id: i32,
    /// 是否已取消。
    pub cancelled: bool,
}

impl Event for WindowButtonClickEvent {}

impl EntityEvent for WindowButtonClickEvent {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for WindowButtonClickEvent {}

impl CancellableEvent for WindowButtonClickEvent {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
