//! 插件消息事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 插件消息事件（serverbound `CustomPayload`）。
#[derive(Message, Debug, Clone)]
pub struct PluginMessageEvent {
    /// 发送消息的玩家实体。
    pub player: Entity,
    /// 消息频道名称。
    pub channel: String,
    /// 消息数据。
    pub data: Vec<u8>,
    /// 是否已取消。
    pub cancelled: bool,
}

impl Event for PluginMessageEvent {}

impl EntityEvent for PluginMessageEvent {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for PluginMessageEvent {}

impl CancellableEvent for PluginMessageEvent {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
