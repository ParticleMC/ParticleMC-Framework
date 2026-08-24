//! 实体死亡事件。

use crate::event::r#trait::{EntityEvent, Event};
use crate::prelude::{Entity, Message};

/// 实体死亡事件。
#[derive(Message, Debug, Clone)]
pub struct EntityDeath {
    /// 死亡实体。
    pub entity: Entity,
}

impl Event for EntityDeath {}

impl EntityEvent for EntityDeath {
    fn entity(&self) -> Entity {
        self.entity
    }
}
