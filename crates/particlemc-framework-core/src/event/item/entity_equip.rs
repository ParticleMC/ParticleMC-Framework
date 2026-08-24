//! 实体装备事件。

use super::EquipmentSlot;
use crate::event::r#trait::{EntityEvent, Event};
use crate::prelude::{Entity, Message};

/// 实体装备事件。
#[derive(Message, Debug, Clone)]
pub struct EntityEquip {
    /// 实体。
    pub entity: Entity,
    /// 装备槽位。
    pub slot: EquipmentSlot,
    /// 装备物品。
    pub item: String,
}

impl Event for EntityEquip {}

impl EntityEvent for EntityEquip {
    fn entity(&self) -> Entity {
        self.entity
    }
}
