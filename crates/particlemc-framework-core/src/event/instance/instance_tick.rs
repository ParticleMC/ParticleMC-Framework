//! 实例 tick 事件。

use crate::event::r#trait::{Event, InstanceEvent};
use crate::prelude::Message;
use particlemc_framework_ecs::scheduler::WorldId;

/// 实例 tick 事件。
#[derive(Message, Debug, Clone)]
pub struct InstanceTick {
    /// 实例世界 id。
    pub instance_id: WorldId,
    /// tick 计数。
    pub tick_count: u64,
}

impl Event for InstanceTick {}

impl InstanceEvent for InstanceTick {
    fn instance_id(&self) -> Option<WorldId> {
        Some(self.instance_id)
    }
}
