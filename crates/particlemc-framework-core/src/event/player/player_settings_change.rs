//! 玩家设置变更事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 聊天模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatMode {
    /// 仅聊天。
    ChatOnly,
    /// 命令仅聊天。
    CommandsOnly,
    /// 全部。
    All,
}

/// 玩家设置变更事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerSettingsChange {
    /// 玩家实体。
    pub player: Entity,
    /// 区域设置。
    pub locale: String,
    /// 视距。
    pub view_distance: u8,
    /// 聊天模式。
    pub chat_mode: ChatMode,
    /// 是否启用聊天颜色。
    pub chat_colors: bool,
    /// 是否已取消。
    pub cancelled: bool,
}

impl Event for PlayerSettingsChange {}

impl EntityEvent for PlayerSettingsChange {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for PlayerSettingsChange {}

impl CancellableEvent for PlayerSettingsChange {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
