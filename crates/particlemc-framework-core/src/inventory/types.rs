//! 容器类型系统：抽象 `Inventory` trait、通用 `AbstractInventory` 基础实现与
//! 各具体方块容器（箱子/熔炉/铁砧/信标/酿造台/附魔台/村民）。
//!
//! 语义对齐 Java `net.minestom.server.inventory.AbstractInventory`（size/越界静默
//! 忽略/非 AIR 标脏）与 `inventory/type/`（Furnace 3 槽 + 进度、Anvil 3 槽、
//! Beacon 1 槽、BrewingStand 药水槽、EnchantmentTable 2 槽、Villager 3 槽）。
//! 本任务只对齐语义，不复制翻译 Java。
//!
//! 变更标识符：`complete-missing-subsystems`（R10 容器类型系统）。

use std::collections::HashSet;

use crate::item_stack::ItemStack;

/// 通用容器库存抽象。
///
/// 具体库存（玩家背包、各类方块容器）通过实现该 trait 提供统一的槽位读写、
/// 脏槽跟踪与光标语义。默认光标为空（`get_cursor` 返回 [`ItemStack::AIR`]，
/// `set_cursor` 为无操作）。
pub trait Inventory: Send + Sync {
    /// 槽位总数。
    fn size(&self) -> usize;
    /// 读取指定槽位的物品；越界返回 [`ItemStack::AIR`]。
    fn get_item(&self, slot: usize) -> ItemStack;
    /// 写入指定槽位的物品；非 AIR 写入应将该槽标记为脏（越界静默忽略）。
    fn set_item(&mut self, slot: usize, item: ItemStack);
    /// 当前待下发的脏槽集合（非 AIR 写入产生）。
    fn dirty_slots(&self) -> Vec<u8>;
    /// 清空脏槽集合（同步消费后调用）。
    fn clear_dirty(&mut self);
    /// 读取光标物品（默认无光标）。
    fn get_cursor(&self) -> ItemStack {
        ItemStack::AIR
    }
    /// 设置光标物品（默认忽略）。
    fn set_cursor(&mut self, _item: ItemStack) {}
}

/// 通用库存基础实现：以 `Vec<ItemStack>` 维护槽位 + `HashSet<u8>` 跟踪脏槽。
///
/// 越界读写静默忽略（读返回 [`ItemStack::AIR`]、写无效果）；非 AIR 写入标记脏槽。
/// 语义对齐 Java `AbstractInventory`（size/越界/脏槽）。
#[derive(Clone, Debug)]
pub struct AbstractInventory {
    slots: Vec<ItemStack>,
    dirty: HashSet<u8>,
    size: usize,
}

impl AbstractInventory {
    /// 构造给定槽位数量的空库存（全 AIR、脏槽为空）。
    pub fn new(size: usize) -> Self {
        Self {
            slots: vec![ItemStack::AIR; size],
            dirty: HashSet::new(),
            size,
        }
    }

    /// 槽位总数。
    pub fn size(&self) -> usize {
        self.size
    }

    /// 读取指定槽位；越界返回 AIR（禁止裸 `[i]` 索引，用 `get`）。
    pub fn get_item(&self, slot: usize) -> ItemStack {
        self.slots.get(slot).cloned().unwrap_or(ItemStack::AIR)
    }

    /// 写入指定槽位；越界静默忽略，非 AIR 写入标记脏槽。
    pub fn set_item(&mut self, slot: usize, item: ItemStack) {
        let is_air = item.is_air();
        if let Some(target) = self.slots.get_mut(slot) {
            *target = item;
            if !is_air {
                // slot 已被 get_mut 校验为合法索引（< size），u8 缩窄恒成功；
                // 仍用 TryFrom 兜底以防未来 size 常量变动，失败则静默忽略。
                if let Ok(idx) = u8::try_from(slot) {
                    self.dirty.insert(idx);
                }
            }
        }
    }

    /// 当前待下发的脏槽集合。
    pub fn dirty_slots(&self) -> Vec<u8> {
        self.dirty.iter().copied().collect()
    }

    /// 清空脏槽集合。
    pub fn clear_dirty(&mut self) {
        self.dirty.clear();
    }
}

impl Inventory for AbstractInventory {
    fn size(&self) -> usize {
        self.size()
    }

    fn get_item(&self, slot: usize) -> ItemStack {
        self.get_item(slot)
    }

    fn set_item(&mut self, slot: usize, item: ItemStack) {
        self.set_item(slot, item);
    }

    fn dirty_slots(&self) -> Vec<u8> {
        self.dirty_slots()
    }

    fn clear_dirty(&mut self) {
        self.clear_dirty();
    }
}

/// 为组合 `inner: AbstractInventory` 字段的容器类型生成 `Inventory` trait 委托实现。
///
/// 槽位读写/脏槽全部委托给内层 `AbstractInventory`；光标沿用 trait 默认（无光标）。
macro_rules! delegate_inventory {
    ($ty:ty) => {
        impl Inventory for $ty {
            fn size(&self) -> usize {
                self.inner.size()
            }

            fn get_item(&self, slot: usize) -> ItemStack {
                self.inner.get_item(slot)
            }

            fn set_item(&mut self, slot: usize, item: ItemStack) {
                self.inner.set_item(slot, item);
            }

            fn dirty_slots(&self) -> Vec<u8> {
                self.inner.dirty_slots()
            }

            fn clear_dirty(&mut self) {
                self.inner.clear_dirty();
            }
        }
    };
}

/// 箱子容器。槽位数通常为 9/27/54（单/双/大箱子），可任意指定。
///
/// 语义对齐 Java `InventoryType.CHEST_*`（1/2/3/4/5/6 行 = 9/18/27/36/45/54 槽）。
pub struct ChestInventory {
    inner: AbstractInventory,
}

impl ChestInventory {
    /// 构造给定槽位数的空箱子容器（常见 9/27/54）。
    pub fn new(size: usize) -> Self {
        Self {
            inner: AbstractInventory::new(size),
        }
    }
}
delegate_inventory!(ChestInventory);

/// 熔炉容器：3 槽（0 原料 / 1 燃料 / 2 产物）+ 烧炼进度。
///
/// 语义对齐 Java `FurnaceInventory`（`InventoryType.FURNACE` 3 槽 +
/// 剩余燃料/最大燃烧/进度/最大进度字段）。
pub struct FurnaceInventory {
    inner: AbstractInventory,
    /// 燃料剩余燃烧 tick（`burn_time`，对齐 Java `remainingFuelTick`）。
    burn_time: u32,
    /// 当前烧炼进度 tick（`cook_time`，对齐 Java `progressArrow`）。
    cook_time: u32,
}

impl FurnaceInventory {
    /// 构造空熔炉（3 槽 + 进度为 0）。
    pub fn new() -> Self {
        Self {
            inner: AbstractInventory::new(3),
            burn_time: 0,
            cook_time: 0,
        }
    }

    /// 燃料剩余燃烧 tick。
    pub fn get_burn_time(&self) -> u32 {
        self.burn_time
    }

    /// 设置燃料剩余燃烧 tick。
    pub fn set_burn_time(&mut self, burn_time: u32) {
        self.burn_time = burn_time;
    }

    /// 当前烧炼进度 tick。
    pub fn get_cook_time(&self) -> u32 {
        self.cook_time
    }

    /// 设置当前烧炼进度 tick。
    pub fn set_cook_time(&mut self, cook_time: u32) {
        self.cook_time = cook_time;
    }
}

impl Default for FurnaceInventory {
    /// 默认构造（空熔炉）。
    fn default() -> Self {
        Self::new()
    }
}
delegate_inventory!(FurnaceInventory);

/// 铁砧容器：3 槽（0 左物品 / 1 右物品 / 2 结果）+ 修理费用。
///
/// 语义对齐 Java `AnvilInventory`（`InventoryType.ANVIL` 3 槽）。
pub struct AnvilInventory {
    inner: AbstractInventory,
    /// 铁砧修理费用（经验等级）。
    repair_cost: u32,
}

impl AnvilInventory {
    /// 构造空铁砧（3 槽 + 费用为 0）。
    pub fn new() -> Self {
        Self {
            inner: AbstractInventory::new(3),
            repair_cost: 0,
        }
    }

    /// 铁砧修理费用（经验等级）。
    pub fn get_repair_cost(&self) -> u32 {
        self.repair_cost
    }

    /// 设置铁砧修理费用（经验等级）。
    pub fn set_repair_cost(&mut self, repair_cost: u32) {
        self.repair_cost = repair_cost;
    }
}

impl Default for AnvilInventory {
    /// 默认构造（空铁砧）。
    fn default() -> Self {
        Self::new()
    }
}
delegate_inventory!(AnvilInventory);

/// 信标容器：1 槽（放入物品）。
///
/// 语义对齐 Java `BeaconInventory`（`InventoryType.BEACON` 1 槽）。
pub struct BeaconInventory {
    inner: AbstractInventory,
}

impl BeaconInventory {
    /// 构造空信标（1 槽）。
    pub fn new() -> Self {
        Self {
            inner: AbstractInventory::new(1),
        }
    }
}

impl Default for BeaconInventory {
    /// 默认构造（空信标）。
    fn default() -> Self {
        Self::new()
    }
}
delegate_inventory!(BeaconInventory);

/// 酿造台容器：4 槽（0-2 药水 / 3 燃料）。
///
/// 语义对齐 Java `BrewingStandInventory`（`InventoryType.BREWING_STAND`，
/// 本实现按任务规格采用 4 槽：药水 + 燃料）。
pub struct BrewingStandInventory {
    inner: AbstractInventory,
}

impl BrewingStandInventory {
    /// 构造空酿造台（4 槽：0-2 药水 / 3 燃料）。
    pub fn new() -> Self {
        Self {
            inner: AbstractInventory::new(4),
        }
    }
}

impl Default for BrewingStandInventory {
    /// 默认构造（空酿造台）。
    fn default() -> Self {
        Self::new()
    }
}
delegate_inventory!(BrewingStandInventory);

/// 附魔台容器：2 槽 + 附魔随机种子。
///
/// 语义对齐 Java `EnchantmentTableInventory`（`InventoryType.ENCHANTMENT_TABLE`
/// 2 槽）。
pub struct EnchantmentTableInventory {
    inner: AbstractInventory,
    /// 附魔随机种子（决定附魔选项）。
    enchantment_seed: u32,
}

impl EnchantmentTableInventory {
    /// 构造空附魔台（2 槽 + 种子为 0）。
    pub fn new() -> Self {
        Self {
            inner: AbstractInventory::new(2),
            enchantment_seed: 0,
        }
    }

    /// 附魔随机种子。
    pub fn get_enchantment_seed(&self) -> u32 {
        self.enchantment_seed
    }

    /// 设置附魔随机种子。
    pub fn set_enchantment_seed(&mut self, enchantment_seed: u32) {
        self.enchantment_seed = enchantment_seed;
    }
}

impl Default for EnchantmentTableInventory {
    /// 默认构造（空附魔台）。
    fn default() -> Self {
        Self::new()
    }
}
delegate_inventory!(EnchantmentTableInventory);

/// 村民（商人）容器：3 槽 + 商人等级。
///
/// 语义对齐 Java `VillagerInventory`（`InventoryType.MERCHANT` 3 槽）。
pub struct VillagerInventory {
    inner: AbstractInventory,
    /// 商人等级（影响交易解锁与价格）。
    merchant_level: u32,
}

impl VillagerInventory {
    /// 构造空村民容器（3 槽 + 等级为 0）。
    pub fn new() -> Self {
        Self {
            inner: AbstractInventory::new(3),
            merchant_level: 0,
        }
    }

    /// 商人等级。
    pub fn get_merchant_level(&self) -> u32 {
        self.merchant_level
    }

    /// 设置商人等级。
    pub fn set_merchant_level(&mut self, merchant_level: u32) {
        self.merchant_level = merchant_level;
    }
}

impl Default for VillagerInventory {
    /// 默认构造（空村民容器）。
    fn default() -> Self {
        Self::new()
    }
}
delegate_inventory!(VillagerInventory);

/// 容器类型枚举：区分各方块容器的协议语义与槽位数量。
///
/// `Chest` 携带槽位数；其余变体槽位数量固定（与对应容器构造一致）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerType {
    /// 箱子（携带槽位数，常见 9/27/54）。
    Chest(usize),
    /// 熔炉（3 槽）。
    Furnace,
    /// 铁砧（3 槽）。
    Anvil,
    /// 信标（1 槽）。
    Beacon,
    /// 酿造台（4 槽）。
    BrewingStand,
    /// 附魔台（2 槽）。
    EnchantmentTable,
    /// 村民/商人（3 槽）。
    Villager,
}

impl ContainerType {
    /// 该容器类型的槽位数量。
    pub fn size(&self) -> usize {
        match self {
            ContainerType::Chest(size) => *size,
            ContainerType::Furnace => 3,
            ContainerType::Anvil => 3,
            ContainerType::Beacon => 1,
            ContainerType::BrewingStand => 4,
            ContainerType::EnchantmentTable => 2,
            ContainerType::Villager => 3,
        }
    }
}

/// 容器窗口：以协议 window id + 动态 `Inventory` 承载一个已打开的容器。
///
/// 供容器打开/关闭（R13 容器同步）消费：`container_id` 即协议 `WindowId`，
/// `inventory` 为被打开的容器对象。
pub struct ContainerWindow {
    /// 协议窗口 id（`OpenWindow` 包使用）。
    pub container_id: u8,
    /// 被打开的容器。
    pub inventory: Box<dyn Inventory>,
}

impl ContainerWindow {
    /// 构造容器窗口。
    pub fn new(container_id: u8, inventory: Box<dyn Inventory>) -> Self {
        Self {
            container_id,
            inventory,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item_stack::ItemStack;

    /// 非 AIR 测试物品（material=264 钻石, amount=1）。
    fn diamond() -> ItemStack {
        ItemStack::new(264, 1)
    }

    #[test]
    fn abstract_inventory_out_of_bounds_silent() {
        let mut inv = AbstractInventory::new(3);
        assert_eq!(inv.size(), 3);
        // 越界读返回 AIR、越界写静默忽略（不标脏、不 panic）。
        assert!(inv.get_item(99).is_air());
        inv.set_item(99, diamond());
        assert!(inv.dirty_slots().is_empty());
        assert_eq!(inv.size(), 3);
    }

    #[test]
    fn abstract_inventory_set_get_and_dirty() {
        let mut inv = AbstractInventory::new(3);
        inv.set_item(0, diamond());
        assert_eq!(inv.get_item(0), diamond());
        assert_eq!(inv.dirty_slots(), vec![0u8]);
        // AIR 写入不标脏。
        inv.clear_dirty();
        inv.set_item(0, ItemStack::AIR);
        assert!(inv.dirty_slots().is_empty());
        assert!(inv.get_item(0).is_air());
    }

    #[test]
    fn chest_read_write_and_dirty() {
        let mut inv = ChestInventory::new(27);
        assert_eq!(inv.size(), 27);
        inv.set_item(0, diamond());
        assert_eq!(inv.get_item(0), diamond());
        assert_eq!(inv.dirty_slots(), vec![0u8]);
        inv.clear_dirty();
        assert!(inv.dirty_slots().is_empty());
    }

    #[test]
    fn chest_air_write_not_dirty() {
        let mut inv = ChestInventory::new(27);
        inv.set_item(0, diamond());
        inv.clear_dirty();
        inv.set_item(0, ItemStack::AIR);
        assert!(inv.dirty_slots().is_empty());
    }

    #[test]
    fn furnace_three_slots_and_progress() {
        let mut inv = FurnaceInventory::new();
        assert_eq!(inv.size(), 3);
        // 0 原料 / 1 燃料 / 2 产物。
        inv.set_item(0, ItemStack::new(1, 5)); // 石头（原料）
        inv.set_item(1, ItemStack::new(263, 1)); // 煤炭（燃料）
        inv.set_item(2, ItemStack::new(265, 1)); // 铁锭（产物）
        assert_eq!(inv.get_item(0), ItemStack::new(1, 5));
        assert_eq!(inv.get_item(1), ItemStack::new(263, 1));
        assert_eq!(inv.get_item(2), ItemStack::new(265, 1));
        assert_eq!(inv.dirty_slots().len(), 3);
        inv.set_burn_time(200);
        inv.set_cook_time(80);
        assert_eq!(inv.get_burn_time(), 200);
        assert_eq!(inv.get_cook_time(), 80);
    }

    #[test]
    fn anvil_three_slots_and_repair_cost() {
        let mut inv = AnvilInventory::new();
        assert_eq!(inv.size(), 3);
        inv.set_item(0, ItemStack::new(276, 1)); // 左：钻石剑
        inv.set_item(1, ItemStack::new(264, 2)); // 右：钻石
        assert_eq!(inv.get_item(1), ItemStack::new(264, 2));
        inv.set_repair_cost(5);
        assert_eq!(inv.get_repair_cost(), 5);
    }

    #[test]
    fn container_construction_sizes() {
        assert_eq!(ChestInventory::new(9).size(), 9);
        assert_eq!(ChestInventory::new(54).size(), 54);
        assert_eq!(BeaconInventory::new().size(), 1);
        assert_eq!(BrewingStandInventory::new().size(), 4);
        assert_eq!(EnchantmentTableInventory::new().size(), 2);
        assert_eq!(VillagerInventory::new().size(), 3);
    }

    #[test]
    fn brewing_stand_potion_and_fuel_slots() {
        let mut inv = BrewingStandInventory::new();
        // 0-2 药水 / 3 燃料。
        inv.set_item(0, ItemStack::new(373, 1)); // 药水
        inv.set_item(3, ItemStack::new(370, 1)); // 烈焰粉
        assert_eq!(inv.get_item(3), ItemStack::new(370, 1));
    }

    #[test]
    fn enchantment_table_seed_and_villager_level() {
        let mut enchant = EnchantmentTableInventory::new();
        enchant.set_item(0, ItemStack::new(264, 1));
        enchant.set_enchantment_seed(1234);
        assert_eq!(enchant.get_enchantment_seed(), 1234);

        let mut villager = VillagerInventory::new();
        villager.set_item(1, ItemStack::new(388, 3)); // 绿宝石
        villager.set_merchant_level(3);
        assert_eq!(villager.get_merchant_level(), 3);
    }

    #[test]
    fn container_type_size_matches() {
        assert_eq!(ContainerType::Chest(27).size(), 27);
        assert_eq!(ContainerType::Chest(54).size(), 54);
        assert_eq!(ContainerType::Furnace.size(), 3);
        assert_eq!(ContainerType::Anvil.size(), 3);
        assert_eq!(ContainerType::Beacon.size(), 1);
        assert_eq!(ContainerType::BrewingStand.size(), 4);
        assert_eq!(ContainerType::EnchantmentTable.size(), 2);
        assert_eq!(ContainerType::Villager.size(), 3);
    }

    #[test]
    fn container_window_holds_dynamic_inventory() {
        let win = ContainerWindow::new(5, Box::new(ChestInventory::new(27)));
        assert_eq!(win.container_id, 5);
        assert_eq!(win.inventory.size(), 27);
    }
}
