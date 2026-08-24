//! 容器类型系统：抽象 `Inventory` trait、通用 `AbstractInventory` 基础实现与
//! 各具体方块容器（箱子/熔炉/铁砧/信标/酿造台/附魔台/村民）。
//!
//! - [`Inventory`]：容器统一抽象（槽位读写 / 脏槽 / 光标）
//! - [`AbstractInventory`]：通用基础实现（越界静默、非 AIR 标脏）
//! - [`ChestInventory`] / [`FurnaceInventory`] / [`AnvilInventory`] /
//!   [`BeaconInventory`] / [`BrewingStandInventory`] / [`EnchantmentTableInventory`] /
//!   [`VillagerInventory`]：具体方块容器
//! - [`ContainerType`] / [`ContainerWindow`]：容器类型与窗口承载辅助
//!
//! 语义对齐 Java `net.minestom.server.inventory.AbstractInventory` 与
//! `inventory/type/`（本任务只对齐语义，不复制翻译 Java）。
//!
//! 变更标识符：`complete-missing-subsystems`（R10 容器类型系统）。

pub mod types;

pub use types::{
    AbstractInventory, AnvilInventory, BeaconInventory, BrewingStandInventory, ChestInventory,
    ContainerType, ContainerWindow, EnchantmentTableInventory, FurnaceInventory, Inventory,
    VillagerInventory,
};
