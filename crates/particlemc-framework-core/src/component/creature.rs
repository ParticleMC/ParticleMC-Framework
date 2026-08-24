//! 生物实体组件（T4 实体类层级：以组件组合替代 Java `EntityCreature` 继承）。
//!
//! [`EntityCreature`] 标记可移动、可被 AI 控制的生物实体。导航语义由 T6
//! （批 3）的 `Navigator` 接管——本任务先以数值型 `navigation_target` 字段
//! 承载 AI 可写的目标坐标，T6 实现后由编排者替换为 `Navigator` 句柄。
//!
//! 变更标识符：`complete-missing-subsystems`（T4）。

use crate::prelude::Component;

/// 生物实体组件（可移动、可被 AI 控制）。
#[derive(Component, Clone, Copy, Debug, PartialEq, Default)]
#[component(storage = "sparse")]
pub struct EntityCreature {
    /// 当前导航目标坐标（方块坐标，`None` 表示无目标）。
    ///
    /// T4 先行以 `[f64; 3]` 承载，供 AI 直接写入；T6 由编排者替换为
    /// `Navigator` 类型并升级寻路语义。
    pub navigation_target: Option<[f64; 3]>,
}

impl EntityCreature {
    /// 构造无导航目标的生物。
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::prelude::World;

    #[test]
    fn new_has_no_navigation_target() {
        let creature = EntityCreature::new();
        assert_eq!(creature.navigation_target, None);
    }

    #[test]
    fn navigation_target_field_writable() {
        let mut creature = EntityCreature::new();
        creature.navigation_target = Some([10.0, 64.0, -20.0]);
        assert_eq!(creature.navigation_target, Some([10.0, 64.0, -20.0]));
    }

    #[test]
    fn creature_component_can_be_spawned() {
        let mut world = World::new();
        let entity = world.spawn_bundle(EntityCreature::default()).id();
        assert!(world.get::<EntityCreature>(entity).is_some());
    }
}
