//! 拾取经验球事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 拾取经验球事件。
#[derive(Message, Debug, Clone)]
pub struct PickupExperience {
    /// 玩家实体。
    pub player: Entity,
    /// 经验球实体。
    pub experience_orb: Entity,
    /// 经验值数量。
    pub amount: u32,
    /// 是否已取消。
    pub cancelled: bool,
}

impl Event for PickupExperience {}

impl EntityEvent for PickupExperience {
    fn entity(&self) -> Entity {
        self.player
    }
}

impl PlayerEvent for PickupExperience {}

impl CancellableEvent for PickupExperience {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
