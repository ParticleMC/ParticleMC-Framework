//! 实例引用组件。
//!
//! 指向实体所属 `Instance` 的 `WorldId`，用于在 ECS 查询中快速定位实例世界
//! （经 `InstanceScheduler` 访问其实例 World 的 `ChunkStore` 等资源）。
//! 见 `.specs/implement-custom-ecs/spec.md` R11。

use crate::prelude::Component;

use particlemc_framework_ecs::scheduler::WorldId;

/// 实体所属实例的引用。
#[derive(Default, Component, Debug, Clone, Copy, PartialEq, Eq)]
#[component(storage = "sparse")]
pub struct InstanceRef(pub WorldId);
