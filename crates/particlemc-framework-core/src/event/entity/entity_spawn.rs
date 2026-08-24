//! 实体生成事件（升级现有定义）。

use crate::component::Position;
use crate::event::r#trait::{EntityEvent, Event, InstanceEvent};
use crate::prelude::{Entity, Message};
use crate::resource::EntityType;
use particlemc_framework_ecs::scheduler::WorldId;

/// 实体生成事件。
#[derive(Message, Debug, Clone)]
pub struct EntitySpawn {
    /// 生成的实体。
    pub entity: Entity,
    /// 实体类型。
    pub entity_type: EntityType,
    /// 生成位置。
    pub position: Position,
}

impl Event for EntitySpawn {}

impl EntityEvent for EntitySpawn {
    fn entity(&self) -> Entity {
        self.entity
    }
}

impl InstanceEvent for EntitySpawn {
    fn instance_id(&self) -> Option<WorldId> {
        None // 由外部设置
    }
}
