//! 玩家结束使用物品事件。

use crate::event::item::Hand;
use crate::event::r#trait::{EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 玩家结束使用物品事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerFinishItemUse {
    /// 玩家实体。
    pub player: Entity,
    /// 使用的手。
    pub hand: Hand,
    /// 剩余使用 tick 数。
    pub remaining_ticks: u32,
}

impl Event for PlayerFinishItemUse {}

impl EntityEvent for PlayerFinishItemUse {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for PlayerFinishItemUse {}
