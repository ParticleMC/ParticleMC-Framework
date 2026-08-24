//! 实例注册事件。

use crate::event::r#trait::{Event, InstanceEvent};
use crate::prelude::Message;
use particlemc_framework_ecs::scheduler::WorldId;

/// 实例注册事件。
#[derive(Message, Debug, Clone)]
pub struct InstanceRegister {
    /// 实例世界 id。
    pub instance_id: WorldId,
}

impl Event for InstanceRegister {}

impl InstanceEvent for InstanceRegister {
    fn instance_id(&self) -> Option<WorldId> {
        Some(self.instance_id)
    }
}
