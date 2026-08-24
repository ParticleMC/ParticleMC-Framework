// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Item 事件定义（7 个）。

pub mod entity_equip;
pub mod item_drop;
pub mod pickup_experience;
pub mod pickup_item;
pub mod player_begin_item_use;
pub mod player_cancel_item_use;
pub mod player_finish_item_use;

pub use entity_equip::EntityEquip;
pub use item_drop::ItemDrop;
pub use pickup_experience::PickupExperience;
pub use pickup_item::PickupItem;
pub use player_begin_item_use::{Hand, PlayerBeginItemUse};
pub use player_cancel_item_use::PlayerCancelItemUse;
pub use player_finish_item_use::PlayerFinishItemUse;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::r#trait::PlayerEvent;
    use crate::prelude::Entity;

    #[test]
    fn player_event_traits_impl() {
        let evt = PickupItem {
            player: Entity::from_raw_u32(1),
            item: Entity::from_raw_u32(2),
            distance: 1.5,
            cancelled: false,
        };
        assert_eq!(evt.player(), Entity::from_raw_u32(1));
    }
}

/// 装备槽位枚举。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EquipmentSlot {
    /// 头部。
    Head,
    /// 胸部。
    Chest,
    /// 腿部。
    Legs,
    /// 脚部。
    Feet,
    /// 主手。
    MainHand,
    /// 副手。
    OffHand,
}
