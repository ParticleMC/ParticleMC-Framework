// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 掉落物实体组件（T4 实体类层级：以组件组合替代 Java `ItemEntity` 继承）。
//!
//! [`ItemEntity`] 承载地面物品栈；拾取 / 合并语义由后续 tick 系统消费
//! （对齐 Java `ItemEntity` 的 pickable / mergeable 行为，本任务仅定义数据）。
//!
//! 变更标识符：`complete-missing-subsystems`（T4）。

use crate::prelude::Component;

use crate::item_stack::ItemStack;

/// 掉落物实体组件。
#[derive(Default, Component, Clone, Debug, PartialEq)]
#[component(storage = "sparse")]
pub struct ItemEntity {
    /// 地面上的物品栈。
    pub item: ItemStack,
}

impl ItemEntity {
    /// 以物品栈构造掉落物。
    pub fn new(item: ItemStack) -> Self {
        Self { item }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::prelude::World;

    #[test]
    fn new_holds_item() {
        let entity = ItemEntity::new(ItemStack::new(264, 5));
        assert_eq!(entity.item, ItemStack::new(264, 5));
        assert_eq!(entity.item.material, 264);
        assert_eq!(entity.item.amount, 5);
    }

    #[test]
    fn item_entity_component_can_be_spawned() {
        let mut world = World::new();
        let entity = world
            .spawn_bundle(ItemEntity::new(ItemStack::new(1, 3)))
            .id();
        let spawned = world.get::<ItemEntity>(entity).unwrap();
        assert_eq!(spawned.item.material, 1);
    }
}
