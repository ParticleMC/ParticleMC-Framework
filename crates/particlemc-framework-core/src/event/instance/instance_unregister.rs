// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 实例取消注册事件。

use crate::event::r#trait::{Event, InstanceEvent};
use crate::prelude::Message;
use particlemc_framework_ecs::scheduler::WorldId;

/// 实例取消注册事件。
#[derive(Message, Debug, Clone)]
pub struct InstanceUnregister {
    /// 实例世界 id。
    pub instance_id: WorldId,
}

impl Event for InstanceUnregister {}

impl InstanceEvent for InstanceUnregister {
    fn instance_id(&self) -> Option<WorldId> {
        Some(self.instance_id)
    }
}
