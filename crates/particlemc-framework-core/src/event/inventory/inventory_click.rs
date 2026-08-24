//! 背包点击事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 点击类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickAction {
    /// 左键点击。
    LeftClick,
    /// 右键点击。
    RightClick,
    /// 拖动。
    Drag,
    /// 其他。
    Other(u8),
}

/// 背包点击事件。
#[derive(Message, Debug, Clone)]
pub struct InventoryClick {
    /// 玩家实体。
    pub player: Entity,
    /// 点击的槽位。
    pub slot: u8,
    /// 光标上的物品。
    pub cursor: Option<String>,
    /// 点击类型。
    pub click_action: ClickAction,
    /// 是否已取消。
    pub cancelled: bool,
}

impl Event for InventoryClick {}

impl EntityEvent for InventoryClick {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for InventoryClick {}

impl CancellableEvent for InventoryClick {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
