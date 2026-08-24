//! 区块卸载事件。

use crate::event::r#trait::{Event, InstanceEvent};
use crate::prelude::Message;
use particlemc_framework_ecs::scheduler::WorldId;

/// 区块卸载事件。
#[derive(Message, Debug, Clone)]
pub struct InstanceChunkUnload {
    /// 实例世界 id。
    pub instance_id: WorldId,
    /// 区块 X 坐标。
    pub chunk_x: i32,
    /// 区块 Z 坐标。
    pub chunk_z: i32,
}

impl Event for InstanceChunkUnload {}

impl InstanceEvent for InstanceChunkUnload {
    fn instance_id(&self) -> Option<WorldId> {
        Some(self.instance_id)
    }
}
