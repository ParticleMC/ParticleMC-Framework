//! 背包预点击事件（在点击处理前触发）。

use crate::event::inventory::inventory_click::ClickAction;
use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 背包预点击事件。
#[derive(Message, Debug, Clone)]
pub struct InventoryPreClick {
    /// 玩家实体。
    pub player: Entity,
    /// 点击的槽位。
    pub slot: u8,
    /// 点击类型。
    pub click_action: ClickAction,
    /// 是否已取消。
    pub cancelled: bool,
}

impl Event for InventoryPreClick {}

impl EntityEvent for InventoryPreClick {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for InventoryPreClick {}

impl CancellableEvent for InventoryPreClick {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
