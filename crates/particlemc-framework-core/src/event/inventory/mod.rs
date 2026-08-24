//! Inventory 事件定义。

pub mod creative_inventory_action_event;
pub mod inventory_click;
pub mod inventory_close;
pub mod inventory_open;
pub mod inventory_pre_click;
pub mod window_button_click_event;

pub use creative_inventory_action_event::CreativeInventoryActionEvent;
pub use inventory_click::InventoryClick;
pub use inventory_close::InventoryClose;
pub use inventory_open::InventoryOpen;
pub use inventory_pre_click::InventoryPreClick;
pub use window_button_click_event::WindowButtonClickEvent;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::r#trait::PlayerEvent;
    use crate::prelude::Entity;

    #[test]
    fn player_event_traits_impl() {
        let evt = InventoryOpen {
            player: Entity::from_raw_u32(1),
            inventory_type: InventoryType::Chest,
            cancelled: false,
        };
        assert_eq!(evt.player(), Entity::from_raw_u32(1));
    }
}

/// 背包类型枚举。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InventoryType {
    /// 玩家自身背包。
    Player,
    /// 箱子。
    Chest,
    /// 熔炉。
    Furnace,
    /// 工作台。
    CraftingTable,
    /// 末影箱。
    EnderChest,
    /// 交易界面。
    VillagerTrade,
    /// 炼药锅。
    BrewingStand,
    /// 铁砧。
    Anvil,
    /// 其他。
    Other(u8),
}
