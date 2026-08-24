//! 实例容器（世界化记录）。
//!
//! R11 重构后，`InstanceContainer` 不再是主 World 的组件，而是持有实例专属
//! [`WorldId`] 的轻量记录，配合 [`crate::instance::ChunkStore`]（实例 World
//! 内的 `Resource`）与 `InstanceScheduler`（持有并 tick 实例 World）共同构成
//! 「一个实例 = 一个独立 World」的模型。见
//! `.specs/implement-custom-ecs/spec.md` R11。

use particlemc_framework_ecs::scheduler::WorldId;

/// 实例容器：指向实例专属 World 的句柄。
///
/// 区块 / 生成器 / 持久化器现作为 [`crate::instance::ChunkStore`] 存于实例
/// World 内；本结构仅记录该实例的 [`WorldId`]，供 `InstanceManager` 与
/// `InstanceRef` 路由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstanceContainer {
    /// 实例专属 World 的 id（由 `InstanceScheduler` 分配并持有）。
    pub world_id: WorldId,
}

impl InstanceContainer {
    /// 构造指向指定 World 的实例记录。
    pub fn new(world_id: WorldId) -> Self {
        Self { world_id }
    }

    /// 返回实例 World 的 id。
    pub fn world_id(&self) -> WorldId {
        self.world_id
    }
}
