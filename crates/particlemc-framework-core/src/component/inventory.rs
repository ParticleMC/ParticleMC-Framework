// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 玩家库存组件与 Minestom 内部槽 → 窗口槽映射。
//!
//! 提供 [`PlayerInventory`] 组件：以 Minestom 内部序维护 46 个物品槽 + 光标 +
//! 手持热键槽，并跟踪待下发的脏槽集合。另提供
//! [`convert_minestom_slot_to_window_slot`]，将 Minestom 内部槽号转换为
//! `WindowItemsPacket` 内容序（窗口槽）。
//!
//! 槽位权威布局（Minestom 内部序）：
//! - `0-8`   热键栏（hotbar）
//! - `9-35`  背包（27 格）
//! - `36-40` 合成格（结果 + 4 输入）
//! - `41-44` 盔甲（Helmet / Chestplate / Leggings / Boots）
//! - `45`    副手（offhand）
//!
//! 见 `.specs/implement-item-inventory/`（物品与物品栏任务规格）与
//! `.specs/complete-partial-framework-capabilities/`（T2：CLONE 创造克隆、
//! QUICK_CRAFT 合成拖拽权威建模）与 `.specs/complete-missing-subsystems/`
//! （T10/R10：`PlayerInventory` 实现 [`crate::inventory::Inventory`] trait）。

use crate::prelude::Component;
use std::collections::HashSet;

use crate::component::player::GameMode;
use crate::inventory::Inventory;
use crate::item_stack::ItemStack;

/// 玩家库存组件。内部按 Minestom 序维护 46 槽 + 光标 + 手持槽 + 脏槽追踪。
#[derive(Component, Clone, Debug)]
#[component(storage = "sparse")]
pub struct PlayerInventory {
    /// 46 槽物品栈。Minestom 内部序：
    /// 0-8 热键栏, 9-35 背包(27), 36-40 合成格(结果+4), 41-44 盔甲(Helmet/Chestplate/Leggings/Boots), 45 副手。
    pub slots: [ItemStack; 46],
    /// 光标（拖拽中）物品。
    pub cursor: ItemStack,
    /// 当前手持热键槽 0-8。
    pub held_slot: u8,
    /// 待下发的脏槽集合（存 Minestom 内部序号）。
    pub dirty: HashSet<u8>,
    /// 是否需要全量回推 WindowItems（含光标）。登录 / 光标变化（点击、关窗清空）时
    /// 置位，由 `inventory_sync` 消费一次后清零——避免每 tick 全量下发（既有缺陷，
    /// 会导致真实 TCP 集成测试 `collect_until` 永不空闲而挂起）。
    pub full_sync: bool,
    /// 玩家游戏模式（默认生存），供 CLONE(3) 创造克隆校验。框架约定与
    /// [`Player::game_mode`] 保持一致：由调用方（如 `network_receive`）同步设置。
    pub game_mode: GameMode,
    /// 合成拖拽（QUICK_CRAFT, 模式 5）跨次点击状态；无拖拽时为默认值。
    pub quick_craft: QuickCraftState,
}

/// `[ItemStack; 46]` 长度超过 32，Rust 不提供数组成员 `Default`；
/// 手写默认复用 [`Self::new`]（全 AIR，保持与登录初始化一致）。
impl Default for PlayerInventory {
    fn default() -> Self {
        Self::new()
    }
}

/// 合成拖拽（QUICK_CRAFT，模式 5）跨次点击状态。
///
/// 阶段（button）语义：0=拖起（用点击槽初始化 `targets` 与 `snapshot`）、
/// 1=放置全部、2=放置一半（向上取整）、3=继续（v1 与阶段 1 相同逻辑，完成后结算）。
/// `stage < 4` 表示拖拽未结算；未结算时收到其他点击或 `drop_cursor`（关窗）将
/// 清除拖拽状态并把 `targets` 槽回滚到 `snapshot`（光标内容保留）。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct QuickCraftState {
    /// 当前阶段（0-3 拖拽中；结算后清除为默认值且 `targets` 为空）。
    pub stage: u8,
    /// 参与拖拽的槽（Minestom 内部序）。
    pub targets: Vec<u8>,
    /// 拖拽前这些槽的快照，供取消时回滚（与 `targets` 等长对齐）。
    pub snapshot: Vec<ItemStack>,
}

#[allow(clippy::new_without_default)]
impl PlayerInventory {
    /// 全 AIR、held_slot=0、dirty 空、full_sync=true（登录后首 tick 全量下发一次）。
    pub fn new() -> Self {
        Self {
            slots: std::array::from_fn(|_| ItemStack::AIR),
            cursor: ItemStack::AIR,
            held_slot: 0,
            dirty: HashSet::new(),
            full_sync: true,
            game_mode: GameMode::Survival,
            quick_craft: QuickCraftState::default(),
        }
    }

    /// 标记需要全量回推（光标变化时调用）。
    fn mark_full_sync(&mut self) {
        self.full_sync = true;
    }

    /// 设置本库存组件维护的游戏模式（框架约定与 [`Player::game_mode`] 保持一致，
    /// 供 CLONE(3) 创造克隆校验）。
    pub fn set_game_mode(&mut self, mode: GameMode) {
        self.game_mode = mode;
    }

    /// 是否创造模式（CLONE(3) 创造克隆的前置校验）。
    pub fn is_creative(&self) -> bool {
        self.game_mode == GameMode::Creative
    }

    /// 取槽；越界安全返回 AIR（必须用 `.get()`，禁止 `[i]` 索引）。
    pub fn get(&self, slot: usize) -> ItemStack {
        match self.slots.get(slot) {
            Some(item) => item.clone(),
            None => ItemStack::AIR,
        }
    }

    /// 设槽；写入非 AIR 物品时将该 slot 加入 dirty。
    /// 越界（get_mut 返回 None）时静默忽略，不 panic。
    pub fn set(&mut self, slot: usize, item: ItemStack) {
        let is_air = item.is_air();
        if let Some(target) = self.slots.get_mut(slot) {
            *target = item;
            if !is_air {
                // slot 已被 get_mut 校验为合法索引（< 46），u8 转换恒成功；
                // 仍用 TryFrom 处理以防未来常数变动，失败则静默忽略。
                if let Ok(idx) = u8::try_from(slot) {
                    self.dirty.insert(idx);
                }
            }
        }
    }

    /// 权威处理一次容器点击（服务端计算，忽略客户端 `changed_slots` / `carried_item` 预测，防作弊与不一致）。
    ///
    /// 参数：`window_slot` 来自客户端 ClickContainer.slot（窗口序 0..=45），
    /// `button` 来自 ClickContainer.button，`mode` 来自 ClickContainer.mode（i32 点击模式）。
    ///
    /// 返回 `true` 表示库存状态变化（产生脏槽，待 `inventory_sync` 下发），`false` 表示无变化或本框架不做权威建模。
    ///
    /// 支持：Pickup(0) 左键整取/放置、右键取半/放一；QuickMove(1) shift 搬运（热键↔背包）；
    /// Swap(2) 与 button 指定的热键槽(0-8)交换；Clone(3) 创造克隆（仅创造模式，
    /// button=0 整堆 / button=1 取半）；Throw(4) 丢弃整堆/单个；QuickCraft(5) 合成拖拽
    /// （button 0-3 阶段：拖起/放置全部/放置一半/继续）；PickupAll(6) 双击收集全部同类入光标。
    /// 不支持（返回 false）：未知 mode / 越界窗口槽。
    ///
    /// 注：合成拖拽未结算（`stage < 4`）时收到非 QUICK_CRAFT 点击会先取消拖拽——
    /// 把 targets 槽回滚到拖拽前快照后再处理本次点击（见 [`QuickCraftState`]）。
    ///
    /// 见 `.specs/implement-item-click/`。
    ///
    /// 注：权威重算中每槽的堆叠上限取自物品 `max_stack_size` 组件
    /// （见 `ItemStack::max_stack`，缺省 64，规范来源 `crate::item_stack::MAX_STACK`），
    /// 详见 `.specs/implement-item-components/`。
    pub fn apply_click(&mut self, window_slot: i32, button: i8, mode: i32) -> bool {
        // 窗口序越界（合法范围 0..=45）直接拒绝。
        if !(0..=45).contains(&window_slot) {
            return false;
        }
        let internal = window_slot_to_minestom_slot(window_slot);
        // internal ∈ [0,45]，缩窄 usize 恒成功；仍用 TryFrom 兜底以防常数变动。
        let slot = match usize::try_from(internal) {
            Ok(s) => s,
            Err(_) => return false,
        };
        let mut changed = false;
        // 非 QUICK_CRAFT 点击会打断未结算的合成拖拽：先回滚 targets 槽到拖拽前快照，
        // 光标保留当前内容（见 `.specs/complete-partial-framework-capabilities/`）。
        if mode != CLICK_QUICK_CRAFT && self.quick_craft_active() {
            changed = self.cancel_quick_craft();
        }
        let click_changed = match mode {
            CLICK_PICKUP => self.click_pickup(slot, button),
            CLICK_QUICK_MOVE => self.click_quick_move(slot),
            CLICK_SWAP => self.click_swap(slot, button),
            CLICK_CLONE => self.click_clone(slot, button),
            CLICK_THROW => self.click_throw(slot, button),
            CLICK_QUICK_CRAFT => self.click_quick_craft(slot, button),
            CLICK_PICKUP_ALL => self.click_pickup_all(),
            // 未知 mode：不做权威建模。
            _ => false,
        };
        changed = changed || click_changed;
        // 点击可能改变光标，标记全量回推（下一 tick 收敛，见 `.specs/implement-framework-capabilities/`）。
        if changed {
            self.mark_full_sync();
        }
        changed
    }

    /// Pickup(0)：左键(button=0)整取/合并/交换；右键(button=1)取半/放一/交换。
    fn click_pickup(&mut self, slot: usize, button: i8) -> bool {
        let slot_stack = self.get(slot);
        let cursor = self.cursor.clone();
        if button == 0 {
            if cursor.is_air() {
                if slot_stack.is_air() {
                    return false;
                }
                self.cursor = slot_stack;
                self.set(slot, ItemStack::AIR);
                return true;
            }
            if slot_stack.is_air() || slot_stack.material == cursor.material {
                let space = slot_stack.max_stack().saturating_sub(slot_stack.amount);
                let to_move = space.min(cursor.amount);
                if to_move == 0 {
                    return false;
                }
                // 以光标材质构造目标槽；to_move ≥ 1 保证数量非零（不会退回 AIR）。
                let new_amount = slot_stack.amount.saturating_add(to_move);
                let new_slot = ItemStack::new(cursor.material, new_amount);
                let mut new_cursor = cursor.clone();
                new_cursor.amount = cursor.amount.saturating_sub(to_move);
                self.set(slot, new_slot);
                self.cursor = if new_cursor.amount == 0 {
                    ItemStack::AIR
                } else {
                    new_cursor
                };
                return true;
            }
            // 不同物品：交换光标与槽
            self.cursor = slot_stack;
            self.set(slot, cursor);
            return true;
        } else if button == 1 {
            if cursor.is_air() {
                if slot_stack.is_air() {
                    return false;
                }
                // 取一半（向上取整）；用 u16 中介避免 u8 溢出，再安全缩窄为 u8（half ≤ 32 恒可容纳）。
                let half =
                    u8::try_from(u16::from(slot_stack.amount).div_ceil(2)).unwrap_or(u8::MAX);
                let mut new_slot = slot_stack.clone();
                new_slot.amount = slot_stack.amount.saturating_sub(half);
                self.cursor = ItemStack::new(slot_stack.material, half);
                self.set(
                    slot,
                    if new_slot.amount == 0 {
                        ItemStack::AIR
                    } else {
                        new_slot
                    },
                );
                return true;
            }
            if slot_stack.is_air() || slot_stack.material == cursor.material {
                if slot_stack.amount < slot_stack.max_stack() && cursor.amount > 0 {
                    // 以光标材质构造目标槽；放置 1 个保证数量非零（不会退回 AIR）。
                    let new_amount = slot_stack.amount.saturating_add(1);
                    let new_slot = ItemStack::new(cursor.material, new_amount);
                    let mut new_cursor = cursor.clone();
                    new_cursor.amount = cursor.amount.saturating_sub(1);
                    self.set(slot, new_slot);
                    self.cursor = if new_cursor.amount == 0 {
                        ItemStack::AIR
                    } else {
                        new_cursor
                    };
                    return true;
                }
                return false;
            }
            self.cursor = slot_stack;
            self.set(slot, cursor);
            return true;
        }
        false
    }

    /// QuickMove(1)：shift 搬运，仅热键(0-8)↔背包(9-35) 互搬；源为合成/盔甲/副手(>=36) 返回 false。
    fn click_quick_move(&mut self, slot: usize) -> bool {
        let stack = self.get(slot);
        if stack.is_air() {
            return false;
        }
        let dest: std::ops::RangeInclusive<usize> = if slot <= 8 {
            9..=35
        } else if (9..=35).contains(&slot) {
            0..=8
        } else {
            return false;
        };
        let mut remaining = stack.amount;
        for d in dest.clone() {
            if remaining == 0 {
                break;
            }
            let dstack = self.get(d);
            if !dstack.is_air()
                && dstack.material == stack.material
                && dstack.amount < stack.max_stack()
            {
                let space = stack.max_stack() - dstack.amount;
                let move_amt = space.min(remaining);
                let mut ns = dstack.clone();
                ns.amount = dstack.amount.saturating_add(move_amt);
                self.set(d, ns);
                remaining = remaining.saturating_sub(move_amt);
            }
        }
        if remaining > 0 {
            for d in dest {
                if remaining == 0 {
                    break;
                }
                if self.get(d).is_air() {
                    self.set(d, ItemStack::new(stack.material, remaining));
                    remaining = 0;
                }
            }
        }
        if remaining == 0 {
            self.set(slot, ItemStack::AIR);
        } else {
            self.set(slot, ItemStack::new(stack.material, remaining));
        }
        remaining != stack.amount
    }

    /// Swap(2)：与 button 指定的热键槽(0-8) 交换；button 越界返回 false。
    fn click_swap(&mut self, slot: usize, button: i8) -> bool {
        if !(0..=8).contains(&button) {
            return false;
        }
        // i8(0..=8) → usize 为拓宽，用 TryFrom 兜底。
        let hotbar = match usize::try_from(button) {
            Ok(h) => h,
            Err(_) => return false,
        };
        let a = self.get(slot);
        let b = self.get(hotbar);
        self.set(slot, b);
        self.set(hotbar, a);
        true
    }

    /// Throw(4)：button=0 丢弃整堆；button=1 丢弃单个；空槽返回 false。
    fn click_throw(&mut self, slot: usize, button: i8) -> bool {
        let stack = self.get(slot);
        if stack.is_air() {
            return false;
        }
        if button == 0 {
            self.set(slot, ItemStack::AIR);
            true
        } else if button == 1 {
            let mut ns = stack.clone();
            ns.amount = stack.amount.saturating_sub(1);
            self.set(slot, if ns.amount == 0 { ItemStack::AIR } else { ns });
            true
        } else {
            false
        }
    }

    /// PickupAll(6)：光标已有物品时，收集库存中所有同类物品入光标（受物品 max_stack_size 组件限制（缺省 64））；空光标/光标已满返回 false。
    fn click_pickup_all(&mut self) -> bool {
        let cursor = self.cursor.clone();
        if cursor.is_air() || cursor.amount >= cursor.max_stack() {
            return false;
        }
        let material = cursor.material;
        let mut space = cursor.max_stack().saturating_sub(cursor.amount);
        let mut changed = false;
        for s in 0..46usize {
            if space == 0 {
                break;
            }
            let stack = self.get(s);
            if stack.is_air() || stack.material != material {
                continue;
            }
            let take = stack.amount.min(space);
            space = space.saturating_sub(take);
            let mut ns = stack.clone();
            ns.amount = stack.amount.saturating_sub(take);
            self.set(s, if ns.amount == 0 { ItemStack::AIR } else { ns });
            changed = true;
        }
        if changed {
            let max = cursor.max_stack();
            let mut new_cursor = cursor;
            new_cursor.amount = max - space;
            self.cursor = new_cursor;
        }
        changed
    }

    /// Clone(3) 创造克隆：仅创造模式生效。
    ///
    /// button=0：把槽内物品**整堆副本**放入光标（覆盖光标）。对齐 Java `ClickType.Middle`
    /// 语义——Java `ClickPreprocessor` 对 `CLONE` 恒产出整堆 Middle 克隆（无取半），
    /// 故仅支持 button=0，其余 button 返回 `false`。
    ///
    /// 槽空或非创造模式返回 `false`，库存与光标不变。
    /// 见 `.specs/complete-partial-framework-capabilities/`（R2）。
    fn click_clone(&mut self, slot: usize, button: i8) -> bool {
        if button != 0 {
            return false;
        }
        if !self.is_creative() {
            return false;
        }
        let stack = self.get(slot);
        if stack.is_air() {
            return false;
        }
        // 整堆副本（保留 components），槽内容不变。
        self.cursor = stack;
        true
    }

    /// QuickCraft(5) 合成拖拽：`button` 即阶段（0-3），`slot` 为窗口槽对应的内部槽。
    ///
    /// 阶段语义：
    /// - 0 拖起：用 `slot` 初始化拖拽（`targets=[slot]` + 拖拽前快照）；光标为 AIR
    ///   时无物可拖，返回 `false`。
    /// - 1 放置全部：把光标物品按 spread 规则分发到 targets 各槽，受 `max_stack`
    ///   限制，剩余回光标。
    /// - 2 放置一半：分发光标数量的向上取整一半，剩余回光标。
    /// - 3 继续：v1 简化为与阶段 1 相同逻辑（spread 光标），完成后结算清除拖拽状态。
    ///
    /// 各阶段会先把点击槽加入 targets（拖过一排的效果），随后按阶段分发。
    /// 返回 `true` 表示状态变化（阶段 0 初始化成功、或实际分发成功），否则 `false`。
    fn click_quick_craft(&mut self, slot: usize, button: i8) -> bool {
        match button {
            0 => self.quick_craft_start(slot),
            1 => self.quick_craft_place_all(slot),
            2 => self.quick_craft_place_half(slot),
            3 => self.quick_craft_continue(slot),
            _ => false,
        }
    }

    /// QUICK_CRAFT 阶段 0（拖起）：初始化 `targets=[slot]` 与拖拽前快照。
    /// 光标为 AIR 时无物可拖，返回 `false` 且不初始化。
    fn quick_craft_start(&mut self, slot: usize) -> bool {
        if self.cursor.is_air() {
            return false;
        }
        let slot_u8 = match u8::try_from(slot) {
            Ok(s) => s,
            Err(_) => return false,
        };
        self.quick_craft = QuickCraftState {
            stage: 0,
            targets: vec![slot_u8],
            snapshot: vec![self.get(slot)],
        };
        true
    }

    /// QUICK_CRAFT 阶段 1（放置全部）：把光标物品按 spread 规则分发到 targets。
    fn quick_craft_place_all(&mut self, slot: usize) -> bool {
        if !self.quick_craft_active() {
            return false;
        }
        self.quick_craft_add_target(slot);
        self.quick_craft.stage = 1;
        let amount = self.cursor.amount;
        self.quick_craft_spread(amount)
    }

    /// QUICK_CRAFT 阶段 2（放置一半）：分发光标数量的向上取整一半，剩余回光标。
    fn quick_craft_place_half(&mut self, slot: usize) -> bool {
        if !self.quick_craft_active() {
            return false;
        }
        self.quick_craft_add_target(slot);
        self.quick_craft.stage = 2;
        // 用 u16 中介避免 u8 溢出，再安全缩窄为 u8（half ≤ 128 恒可容纳）。
        let half = u8::try_from(u16::from(self.cursor.amount).div_ceil(2)).unwrap_or(u8::MAX);
        self.quick_craft_spread(half)
    }

    /// QUICK_CRAFT 阶段 3（继续）：v1 简化为与阶段 1（放置全部）相同逻辑，
    /// 分发后**结算**——清除拖拽状态（不保留）。
    fn quick_craft_continue(&mut self, slot: usize) -> bool {
        if !self.quick_craft_active() {
            return false;
        }
        self.quick_craft_add_target(slot);
        self.quick_craft.stage = 3;
        let amount = self.cursor.amount;
        let changed = self.quick_craft_spread(amount);
        // 结算：阶段 3 完成后清除拖拽状态（不保留）。
        self.quick_craft = QuickCraftState::default();
        changed
    }

    /// 把点击槽加入拖拽 targets（若尚未包含），并记录其拖拽前快照。返回是否新加入。
    fn quick_craft_add_target(&mut self, slot: usize) -> bool {
        let slot_u8 = match u8::try_from(slot) {
            Ok(s) => s,
            Err(_) => return false,
        };
        if self.quick_craft.targets.contains(&slot_u8) {
            return false;
        }
        let snapshot_item = self.get(slot);
        self.quick_craft.targets.push(slot_u8);
        self.quick_craft.snapshot.push(snapshot_item);
        true
    }

    /// 是否存在未结算的合成拖拽（`targets` 非空即视为活动拖拽，结算后为默认值）。
    fn quick_craft_active(&self) -> bool {
        !self.quick_craft.targets.is_empty()
    }

    /// 按 spread 规则把光标中的 `amount` 个物品分发到 targets（顺序逐槽，
    /// 受 `max_stack` 限制，直到光标配额清空或目标满），实际分发量从光标扣除。
    ///
    /// 语义对齐 Java `ClickProcessor.QUICK_CRAFT_SPREAD`（逐槽顺序放置）。
    /// 返回是否产生了实际分发。
    fn quick_craft_spread(&mut self, amount: u8) -> bool {
        if self.cursor.is_air() || amount == 0 {
            return false;
        }
        let material = self.cursor.material;
        let mut remaining = amount;
        let mut changed = false;
        // 先取快照避免迭代中借用冲突（targets 长度 ≤ 46，克隆开销可忽略）。
        let targets: Vec<u8> = self.quick_craft.targets.clone();
        for t in targets {
            if remaining == 0 {
                break;
            }
            let idx = usize::from(t);
            let stack = self.get(idx);
            if !stack.is_air() && stack.material != material {
                continue;
            }
            let space = stack.max_stack().saturating_sub(stack.amount);
            let add = space.min(remaining);
            if add == 0 {
                continue;
            }
            // 空槽以光标材质构造（add ≥ 1 不会退回 AIR）；同材质槽克隆并加量。
            let ns = if stack.is_air() {
                ItemStack::new(material, add)
            } else {
                let mut ns = stack.clone();
                ns.amount = stack.amount.saturating_add(add);
                ns
            };
            self.set(idx, ns);
            remaining = remaining.saturating_sub(add);
            changed = true;
        }
        if changed {
            // 已分发 amount - remaining 个，从光标扣除（数量非零才保留，否则回 AIR）。
            let new_amount = self.cursor.amount.saturating_sub(amount - remaining);
            self.cursor = if new_amount == 0 {
                ItemStack::AIR
            } else {
                let mut c = self.cursor.clone();
                c.amount = new_amount;
                c
            };
        }
        changed
    }

    /// 取消未结算的合成拖拽：把 targets 槽回滚到拖拽前快照，并清除拖拽状态。
    /// 光标内容保留。无活动拖拽时为无操作，返回 `false`。
    fn cancel_quick_craft(&mut self) -> bool {
        if !self.quick_craft_active() {
            return false;
        }
        // 先取快照避免迭代中借用冲突（targets 长度 ≤ 46，克隆开销可忽略）。
        let targets: Vec<u8> = self.quick_craft.targets.clone();
        let snapshot: Vec<ItemStack> = self.quick_craft.snapshot.clone();
        for (target, snapshot) in targets.iter().zip(snapshot.iter()) {
            self.set(usize::from(*target), snapshot.clone());
        }
        self.quick_craft = QuickCraftState::default();
        true
    }

    /// 关闭窗口时清空光标物品；框架暂无掉落实体，直接丢弃（见 `.specs/implement-item-inventory/`）。
    ///
    /// 若存在未结算的合成拖拽（QUICK_CRAFT 阶段 < 4），先取消拖拽——把 targets 槽
    /// 回滚到拖拽前快照、清除拖拽状态（光标内容在清空前保留），随后清空光标。
    ///
    /// 返回 `true` 表示原本持有非空光标（已清空）或取消了拖拽，`false` 表示无任何变化。
    /// 后置条件：调用后 `cursor == ItemStack::AIR` 且拖拽状态已清除。
    pub fn drop_cursor(&mut self) -> bool {
        let mut changed = false;
        if self.quick_craft_active() {
            changed = self.cancel_quick_craft();
        }
        if self.cursor.is_air() {
            return changed;
        }
        self.cursor = ItemStack::AIR;
        self.mark_full_sync();
        true
    }

    /// 设置当前手持热键槽（0-8）。仅当 `slot <= 8` 时置位并返回 `true`；
    /// 否则返回 `false` 且 `held_slot` 不变（见 `.specs/implement-item-inventory/`）。
    pub fn set_held_slot(&mut self, slot: u8) -> bool {
        if slot <= 8 {
            self.held_slot = slot;
            true
        } else {
            false
        }
    }

    /// 将物品加入库存（ADD 语义，权威来源：Java Minestom `AbstractInventory.addItemStack`）。
    ///
    /// 按 `material` + `components` 相等匹配可堆叠槽（忽略 `amount`），
    /// 受各槽 `max_stack` 限制填入部分；随后遍历 `0..=45` 填入空槽。
    /// 经 `set` 写入的槽会被标记脏。返回未能放入的剩余物品栈，全部放入则返回 `AIR`。
    /// 输入为 `AIR` 时为无操作，直接返回 `AIR`，不改动库存。
    ///
    /// 见 `.specs/implement-item-inventory/`（物品与物品栏任务规格）。
    pub fn add_item(&mut self, item: ItemStack) -> ItemStack {
        if item.is_air() {
            return ItemStack::AIR;
        }
        let mut remaining = item.clone();
        // 阶段一：堆叠到已有同款（material+components 相等、非空）槽，受 max_stack 限制。
        for s in 0..46usize {
            if remaining.is_air() {
                break;
            }
            let stack = self.get(s);
            if stack.is_air() {
                continue;
            }
            if stack.material == remaining.material && stack.components == remaining.components {
                let space = stack.max_stack().saturating_sub(stack.amount);
                if space > 0 {
                    let move_amt = space.min(remaining.amount);
                    let mut ns = stack.clone();
                    ns.amount = stack.amount.saturating_add(move_amt);
                    self.set(s, ns);
                    remaining.amount = remaining.amount.saturating_sub(move_amt);
                }
            }
        }
        // 阶段二：填满空槽（0..=45）。
        for s in 0..46usize {
            if remaining.is_air() {
                break;
            }
            if self.get(s).is_air() {
                let cap = remaining.max_stack();
                let move_amt = cap.min(remaining.amount);
                let mut placed = remaining.clone();
                placed.amount = move_amt;
                self.set(s, placed);
                remaining.amount = remaining.amount.saturating_sub(move_amt);
            }
        }
        if remaining.is_air() {
            ItemStack::AIR
        } else {
            remaining
        }
    }
}

/// [`PlayerInventory`] 实现通用 [`Inventory`] trait（R10 容器类型系统）。
///
/// 槽位读写委托既有 `get`/`set`（越界静默、非 AIR 标脏，行为不变），
/// 脏槽取自 `dirty` 集合，光标委托 `cursor` 字段。
impl Inventory for PlayerInventory {
    /// 槽位总数（Minestom 内部序 46 槽）。
    fn size(&self) -> usize {
        46
    }

    /// 委托既有 [`PlayerInventory::get`]（越界返回 AIR）。
    fn get_item(&self, slot: usize) -> ItemStack {
        self.get(slot)
    }

    /// 委托既有 [`PlayerInventory::set`]（越界静默忽略、非 AIR 标脏）。
    fn set_item(&mut self, slot: usize, item: ItemStack) {
        self.set(slot, item);
    }

    /// 当前待下发的脏槽集合（取自 `dirty`）。
    fn dirty_slots(&self) -> Vec<u8> {
        self.dirty.iter().copied().collect()
    }

    /// 清空脏槽集合。
    fn clear_dirty(&mut self) {
        self.dirty.clear();
    }

    /// 委托 `cursor` 字段。
    fn get_cursor(&self) -> ItemStack {
        self.cursor.clone()
    }

    /// 委托 `cursor` 字段。
    fn set_cursor(&mut self, item: ItemStack) {
        self.cursor = item;
    }
}

/// Minestom 内部槽 → 窗口槽（WindowItemsPacket 内容序）。
///
/// 映射规则（权威来源：Java Minestom `PlayerInventoryUtils.convertMinestomSlotToWindowSlot`）：
/// - 热键栏 `0-8`    → `36-44`（`slot + 36`）
/// - 背包   `9-35`   → `9-35`（不变）
/// - 合成格 `36-40`  → `0-4`（`slot - 36`）
/// - 盔甲   `41-44`  → `5-8`（`slot - 36`）；41=Helmet,42=Chestplate,43=Leggings,44=Boots
/// - 副手   `45`     → `45`
/// - 其它（越界/未知）→ 返回 `slot` 原值（恒等式）
pub fn convert_minestom_slot_to_window_slot(slot: i32) -> i32 {
    match slot {
        0..=8 => slot + 36,
        9..=35 => slot,
        36..=40 => slot - 36,
        41..=44 => slot - 36,
        45 => 45,
        _ => slot,
    }
}

/// 点击模式常量（与 `protocol::packets::ClickMode` 的 i32 值对应，避免 component 层反向依赖 protocol）。
pub const CLICK_PICKUP: i32 = 0;
pub const CLICK_QUICK_MOVE: i32 = 1;
pub const CLICK_SWAP: i32 = 2;
pub const CLICK_CLONE: i32 = 3;
pub const CLICK_THROW: i32 = 4;
pub const CLICK_QUICK_CRAFT: i32 = 5;
pub const CLICK_PICKUP_ALL: i32 = 6;

/// 窗口槽 → Minestom 内部槽（`convert_minestom_slot_to_window_slot` 的逆映射）。
///
/// 映射规则（权威来源：Java Minestom `PlayerInventoryUtils.convertWindowSlotToMinestomSlot`）：
/// - 合成/盔甲窗口序 `0-8`  → 内部 `36-44`（`w + 36`）
/// - 背包窗口序 `9-35`      → 不变
/// - 热键窗口序 `36-44`     → 内部 `0-8`（`w - 36`）
/// - 副手/其它              → `45`（逆映射对越界窗口槽保守返回 45，由调用方校验）
///
/// 见 `.specs/implement-item-click/`。
pub fn window_slot_to_minestom_slot(w: i32) -> i32 {
    if (0..=8).contains(&w) {
        w + 36
    } else if (9..=35).contains(&w) {
        w
    } else if (36..=44).contains(&w) {
        w - 36
    } else {
        45
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::component::player::GameMode;
    use crate::item_stack::{ComponentValue, ItemStack};

    /// 非 AIR 测试物品（material=264 钻石, amount=1）。
    fn diamond() -> ItemStack {
        ItemStack::new(264, 1)
    }

    #[test]
    fn new_is_all_air_with_empty_dirty() {
        let inv = PlayerInventory::new();
        assert!(inv.slots.iter().all(|s| s.is_air()));
        assert_eq!(inv.held_slot, 0);
        assert!(inv.cursor.is_air());
        assert!(inv.dirty.is_empty());
    }

    #[test]
    fn convert_hotbar_to_window() {
        assert_eq!(convert_minestom_slot_to_window_slot(0), 36);
        assert_eq!(convert_minestom_slot_to_window_slot(8), 44);
    }

    #[test]
    fn convert_backpack_unchanged() {
        assert_eq!(convert_minestom_slot_to_window_slot(9), 9);
        assert_eq!(convert_minestom_slot_to_window_slot(35), 35);
    }

    #[test]
    fn convert_crafting_and_armor() {
        assert_eq!(convert_minestom_slot_to_window_slot(36), 0);
        assert_eq!(convert_minestom_slot_to_window_slot(40), 4);
        assert_eq!(convert_minestom_slot_to_window_slot(41), 5); // Helmet
        assert_eq!(convert_minestom_slot_to_window_slot(42), 6); // Chestplate
        assert_eq!(convert_minestom_slot_to_window_slot(43), 7); // Leggings
        assert_eq!(convert_minestom_slot_to_window_slot(44), 8); // Boots
    }

    #[test]
    fn convert_offhand_and_unknown() {
        assert_eq!(convert_minestom_slot_to_window_slot(45), 45);
        // 越界：恒等返回原值
        assert_eq!(convert_minestom_slot_to_window_slot(46), 46);
        assert_eq!(convert_minestom_slot_to_window_slot(-1), -1);
    }

    #[test]
    fn set_non_air_marks_dirty_and_stores() {
        let mut inv = PlayerInventory::new();
        let item = diamond();
        inv.set(36, item.clone());
        assert!(inv.dirty.contains(&36));
        assert_eq!(inv.get(36), item);
    }

    #[test]
    fn set_air_does_not_mark_dirty() {
        let mut inv = PlayerInventory::new();
        // 先放入非 AIR，再置回 AIR
        inv.set(10, diamond());
        assert!(inv.dirty.contains(&10));
        inv.dirty.clear();
        inv.set(10, ItemStack::AIR);
        assert!(!inv.dirty.contains(&10));
        assert!(inv.get(10).is_air());
    }

    #[test]
    fn get_out_of_bounds_returns_air_not_panic() {
        let inv = PlayerInventory::new();
        assert!(inv.get(999).is_air());
    }

    #[test]
    fn set_out_of_bounds_silent_ignore() {
        let mut inv = PlayerInventory::new();
        inv.set(999, diamond());
        // 越界写入被静默忽略：dirty 不应包含该（存在的）槽，且原库存不变
        assert!(inv.dirty.is_empty());
    }

    // ============ 窗口槽 ↔ 内部槽逆映射 ============

    #[test]
    fn window_slot_roundtrip_all_46() {
        for w in 0..=45i32 {
            let internal = window_slot_to_minestom_slot(w);
            assert_eq!(
                convert_minestom_slot_to_window_slot(internal),
                w,
                "窗口槽 {w} 往返应回到原值"
            );
        }
    }

    // ============ Pickup(0) ============

    #[test]
    fn apply_pickup_left_picks_up() {
        let mut inv = PlayerInventory::new();
        inv.set(10, ItemStack::new(264, 10));
        assert!(inv.apply_click(10, 0, CLICK_PICKUP));
        assert_eq!(inv.cursor, ItemStack::new(264, 10));
        assert!(inv.get(10).is_air());
    }

    #[test]
    fn apply_pickup_left_merges() {
        let mut inv = PlayerInventory::new();
        inv.cursor = ItemStack::new(264, 5);
        inv.set(10, ItemStack::new(264, 10));
        assert!(inv.apply_click(10, 0, CLICK_PICKUP));
        assert_eq!(inv.get(10), ItemStack::new(264, 15));
        assert!(inv.cursor.is_air());
    }

    #[test]
    fn apply_pickup_left_merges_capped() {
        let mut inv = PlayerInventory::new();
        inv.cursor = ItemStack::new(264, 5);
        inv.set(10, ItemStack::new(264, 62));
        assert!(inv.apply_click(10, 0, CLICK_PICKUP));
        assert_eq!(
            inv.get(10),
            ItemStack::new(264, crate::item_stack::MAX_STACK)
        );
        assert_eq!(inv.cursor, ItemStack::new(264, 3));
    }

    #[test]
    fn apply_pickup_left_swaps_different() {
        let mut inv = PlayerInventory::new();
        inv.cursor = ItemStack::new(1, 5); // 石头
        inv.set(10, ItemStack::new(264, 10)); // 钻石
        assert!(inv.apply_click(10, 0, CLICK_PICKUP));
        assert_eq!(inv.cursor, ItemStack::new(264, 10));
        assert_eq!(inv.get(10), ItemStack::new(1, 5));
    }

    #[test]
    fn apply_pickup_right_takes_half() {
        let mut inv = PlayerInventory::new();
        inv.set(10, ItemStack::new(264, 10));
        assert!(inv.apply_click(10, 1, CLICK_PICKUP));
        assert_eq!(inv.cursor, ItemStack::new(264, 5));
        assert_eq!(inv.get(10), ItemStack::new(264, 5));
    }

    #[test]
    fn apply_pickup_right_takes_half_odd() {
        let mut inv = PlayerInventory::new();
        inv.set(10, ItemStack::new(264, 1));
        assert!(inv.apply_click(10, 1, CLICK_PICKUP));
        assert_eq!(inv.cursor, ItemStack::new(264, 1));
        assert!(inv.get(10).is_air());
    }

    #[test]
    fn apply_pickup_right_places_one() {
        let mut inv = PlayerInventory::new();
        inv.cursor = ItemStack::new(264, 5);
        assert!(inv.apply_click(10, 1, CLICK_PICKUP));
        assert_eq!(inv.get(10), ItemStack::new(264, 1));
        assert_eq!(inv.cursor, ItemStack::new(264, 4));
    }

    #[test]
    fn apply_pickup_right_places_one_full() {
        let mut inv = PlayerInventory::new();
        inv.cursor = ItemStack::new(264, 5);
        inv.set(10, ItemStack::new(264, crate::item_stack::MAX_STACK));
        assert!(!inv.apply_click(10, 1, CLICK_PICKUP));
    }

    #[test]
    fn apply_pickup_right_swaps_different() {
        let mut inv = PlayerInventory::new();
        inv.cursor = ItemStack::new(1, 5); // 石头
        inv.set(10, ItemStack::new(264, 10)); // 钻石
        assert!(inv.apply_click(10, 1, CLICK_PICKUP));
        assert_eq!(inv.cursor, ItemStack::new(264, 10));
        assert_eq!(inv.get(10), ItemStack::new(1, 5));
    }

    #[test]
    fn apply_pickup_invalid_button() {
        let mut inv = PlayerInventory::new();
        inv.set(10, ItemStack::new(264, 10));
        assert!(!inv.apply_click(10, 2, CLICK_PICKUP));
    }

    // ============ QuickMove(1) ============

    #[test]
    fn apply_quick_move_hotbar_to_backpack() {
        let mut inv = PlayerInventory::new();
        inv.set(0, ItemStack::new(264, 10)); // 内部槽 0（热键）
        // 内部热键槽 0 对应窗口槽 36（convert_minestom_slot_to_window_slot(0)=36）。
        assert!(inv.apply_click(36, 0, CLICK_QUICK_MOVE));
        assert!(inv.get(0).is_air());
        // 应落入背包空槽
        let moved = (9..=35).any(|s| inv.get(s) == ItemStack::new(264, 10));
        assert!(moved);
    }

    #[test]
    fn apply_quick_move_backpack_to_hotbar() {
        let mut inv = PlayerInventory::new();
        inv.set(9, ItemStack::new(264, 10)); // 背包槽
        assert!(inv.apply_click(9, 0, CLICK_QUICK_MOVE));
        assert!(inv.get(9).is_air());
        let moved = (0..=8).any(|s| inv.get(s) == ItemStack::new(264, 10));
        assert!(moved);
    }

    #[test]
    fn apply_quick_move_crafting_slot_unsupported() {
        let mut inv = PlayerInventory::new();
        inv.set(36, ItemStack::new(264, 10)); // 内部槽 36（合成格）
        // 内部槽 36 对应窗口槽 0（convert_minestom_slot_to_window_slot(36)=0）。
        assert!(!inv.apply_click(0, 0, CLICK_QUICK_MOVE));
    }

    #[test]
    fn apply_quick_move_empty_source() {
        let mut inv = PlayerInventory::new();
        assert!(!inv.apply_click(0, 0, CLICK_QUICK_MOVE));
    }

    // ============ Swap(2) ============

    #[test]
    fn apply_swap_valid() {
        let mut inv = PlayerInventory::new();
        inv.set(0, ItemStack::new(264, 10)); // 内部槽 0（钻石）
        inv.set(3, ItemStack::new(1, 7)); // 内部热键槽 3（石头）
        // 内部槽 0 对应窗口槽 36；与 button=3 指定的热键槽 3 交换。
        assert!(inv.apply_click(36, 3, CLICK_SWAP));
        assert_eq!(inv.get(0), ItemStack::new(1, 7));
        assert_eq!(inv.get(3), ItemStack::new(264, 10));
    }

    #[test]
    fn apply_swap_invalid_button() {
        let mut inv = PlayerInventory::new();
        inv.set(0, ItemStack::new(264, 10));
        assert!(!inv.apply_click(0, 9, CLICK_SWAP));
    }

    // ============ Throw(4) ============

    #[test]
    fn apply_throw_whole() {
        let mut inv = PlayerInventory::new();
        inv.set(10, ItemStack::new(264, 10));
        assert!(inv.apply_click(10, 0, CLICK_THROW));
        assert!(inv.get(10).is_air());
    }

    #[test]
    fn apply_throw_single() {
        let mut inv = PlayerInventory::new();
        inv.set(10, ItemStack::new(264, 10));
        assert!(inv.apply_click(10, 1, CLICK_THROW));
        assert_eq!(inv.get(10), ItemStack::new(264, 9));
    }

    #[test]
    fn apply_throw_empty() {
        let mut inv = PlayerInventory::new();
        assert!(!inv.apply_click(10, 0, CLICK_THROW));
    }

    // ============ PickupAll(6) ============

    #[test]
    fn apply_pickup_all_collects() {
        let mut inv = PlayerInventory::new();
        inv.cursor = ItemStack::new(264, 1);
        inv.set(9, ItemStack::new(264, 20));
        inv.set(20, ItemStack::new(264, 30));
        inv.set(40, ItemStack::new(264, 13)); // 20+30+13+光标1 = 64，可collect至满
        assert!(inv.apply_click(0, 0, CLICK_PICKUP_ALL));
        assert_eq!(
            inv.cursor,
            ItemStack::new(264, crate::item_stack::MAX_STACK)
        );
        assert!(inv.get(9).is_air());
        assert!(inv.get(20).is_air());
        assert!(inv.get(40).is_air());
    }

    #[test]
    fn apply_pickup_all_empty_cursor() {
        let mut inv = PlayerInventory::new();
        inv.set(9, ItemStack::new(264, 20));
        assert!(!inv.apply_click(0, 0, CLICK_PICKUP_ALL));
    }

    #[test]
    fn apply_pickup_all_cursor_full() {
        let mut inv = PlayerInventory::new();
        inv.cursor = ItemStack::new(264, crate::item_stack::MAX_STACK);
        inv.set(9, ItemStack::new(264, 20));
        assert!(!inv.apply_click(0, 0, CLICK_PICKUP_ALL));
    }

    // ============ T2：每物品堆叠上限（max_stack_size 组件） ============

    /// 末影珍珠（material=368）设 max_stack_size=16；左键整取后再左键放置到已有 10 个的空槽，
    /// 合并后单槽最多 16、光标剩 10，而非 64/26。
    #[test]
    fn apply_pickup_respects_per_item_max_stack() {
        let mut inv = PlayerInventory::new();
        let mut pearl = ItemStack::new(368, 1);
        pearl.components.set(ComponentValue::MaxStackSize(16));
        // 光标持有 16 个末影珍珠
        inv.cursor = ItemStack::new(368, 16);
        // 槽 10 已有 10 个末影珍珠
        let mut in_slot = ItemStack::new(368, 10);
        in_slot.components.set(ComponentValue::MaxStackSize(16));
        inv.set(10, in_slot.clone());
        assert!(inv.apply_click(10, 0, CLICK_PICKUP));
        // 合并后槽 = 10 + 6 = 16（受 max_stack 16 限制），光标剩 10
        let expected_slot = ItemStack::new(368, 16);
        assert_eq!(inv.get(10), expected_slot);
        assert_eq!(inv.cursor, ItemStack::new(368, 10));
    }

    /// 钻石（无 max_stack_size 组件）左键合并仍按 64 上限，保证无回归。
    #[test]
    fn apply_pickup_default_64_without_component() {
        let mut inv = PlayerInventory::new();
        inv.cursor = ItemStack::new(264, 5);
        inv.set(10, ItemStack::new(264, 62));
        assert!(inv.apply_click(10, 0, CLICK_PICKUP));
        assert_eq!(
            inv.get(10),
            ItemStack::new(264, crate::item_stack::MAX_STACK)
        );
        assert_eq!(inv.cursor, ItemStack::new(264, 3));
    }

    // ============ CLONE(3) 创造克隆 ============

    #[test]
    fn clone_creative_copies_whole_stack_to_cursor() {
        let mut inv = PlayerInventory::new();
        inv.set_game_mode(GameMode::Creative);
        inv.set(10, ItemStack::new(264, 10));
        assert!(inv.apply_click(10, 0, CLICK_CLONE));
        // 光标 = 槽整堆副本，槽内容不变。
        assert_eq!(inv.cursor, ItemStack::new(264, 10));
        assert_eq!(inv.get(10), ItemStack::new(264, 10));
    }

    #[test]
    fn clone_creative_overwrites_existing_cursor() {
        let mut inv = PlayerInventory::new();
        inv.set_game_mode(GameMode::Creative);
        inv.cursor = ItemStack::new(1, 5); // 石头
        inv.set(10, ItemStack::new(264, 10)); // 钻石
        assert!(inv.apply_click(10, 0, CLICK_CLONE));
        assert_eq!(
            inv.cursor,
            ItemStack::new(264, 10),
            "克隆应覆盖光标原有物品"
        );
        assert_eq!(inv.get(10), ItemStack::new(264, 10));
    }

    #[test]
    fn clone_creative_button_1_rejected_no_half_clone() {
        let mut inv = PlayerInventory::new();
        inv.set_game_mode(GameMode::Creative);
        inv.set(10, ItemStack::new(264, 10));
        // Java CLONE 语义恒为整堆 Middle 克隆，无取半 → button=1 拒绝。
        assert!(!inv.apply_click(10, 1, CLICK_CLONE));
        assert!(inv.cursor.is_air(), "克隆 button=1 不应改变光标");
        assert_eq!(inv.get(10), ItemStack::new(264, 10));
    }

    #[test]
    fn clone_rejected_in_survival_inventory_unchanged() {
        let mut inv = PlayerInventory::new(); // 默认生存
        inv.set(10, ItemStack::new(264, 10));
        assert!(!inv.apply_click(10, 0, CLICK_CLONE));
        assert!(inv.cursor.is_air(), "非创造模式克隆不应改变光标");
        assert_eq!(inv.get(10), ItemStack::new(264, 10));
    }

    #[test]
    fn clone_empty_slot_noop() {
        let mut inv = PlayerInventory::new();
        inv.set_game_mode(GameMode::Creative);
        assert!(!inv.apply_click(10, 0, CLICK_CLONE));
        assert!(inv.cursor.is_air());
    }

    #[test]
    fn clone_invalid_button_rejected() {
        let mut inv = PlayerInventory::new();
        inv.set_game_mode(GameMode::Creative);
        inv.set(10, ItemStack::new(264, 10));
        assert!(!inv.apply_click(10, 2, CLICK_CLONE));
    }

    // ============ QUICK_CRAFT(5) 合成拖拽 ============

    #[test]
    fn quick_craft_four_stages_spread_and_settle() {
        let mut inv = PlayerInventory::new();
        inv.cursor = ItemStack::new(264, 64);
        inv.set(10, ItemStack::new(264, 60)); // 空间 4
        inv.set(11, ItemStack::new(264, 60)); // 空间 4
        inv.set(12, ItemStack::new(264, 60)); // 空间 4
        // 阶段 0：拖起（窗口槽 10 = 内部槽 10），targets=[10]
        assert!(inv.apply_click(10, 0, CLICK_QUICK_CRAFT));
        assert_eq!(inv.quick_craft.targets, vec![10u8]);
        assert_eq!(inv.cursor, ItemStack::new(264, 64), "拖起不改变光标");
        // 阶段 1：放置全部（窗口槽 11）→ 槽10/11 补满，光标剩 56
        assert!(inv.apply_click(11, 1, CLICK_QUICK_CRAFT));
        assert_eq!(inv.get(10), ItemStack::new(264, 64));
        assert_eq!(inv.get(11), ItemStack::new(264, 64));
        assert_eq!(inv.cursor, ItemStack::new(264, 56));
        // 阶段 2：放置一半（窗口槽 12）→ ceil(56/2)=28 配额，槽12 只余 4 空间 → 补满，
        // 实际分发 4 个，光标剩 52
        assert!(inv.apply_click(12, 2, CLICK_QUICK_CRAFT));
        assert_eq!(inv.get(12), ItemStack::new(264, 64));
        assert_eq!(inv.cursor, ItemStack::new(264, 52));
        // 阶段 3：继续（窗口槽 13，v1 同阶段 1）→ 光标 52 落入空槽 13，结算清除拖拽状态
        assert!(inv.apply_click(13, 3, CLICK_QUICK_CRAFT));
        assert_eq!(inv.get(13), ItemStack::new(264, 52));
        assert!(inv.cursor.is_air());
        assert!(
            inv.quick_craft.targets.is_empty(),
            "阶段 3 完成后应结算清除拖拽状态"
        );
    }

    #[test]
    fn quick_craft_cancel_rolls_back_on_other_click() {
        let mut inv = PlayerInventory::new();
        inv.cursor = ItemStack::new(264, 64);
        inv.set(10, ItemStack::new(264, 60)); // 空间 4
        inv.set(11, ItemStack::new(264, 60)); // 空间 4
        inv.set(20, ItemStack::new(1, 3)); // 石头，与钻石不同
        // 拖起 + 放置全部 → 槽10/11 补满
        assert!(inv.apply_click(10, 0, CLICK_QUICK_CRAFT));
        assert!(inv.apply_click(11, 1, CLICK_QUICK_CRAFT));
        assert_eq!(inv.get(10), ItemStack::new(264, 64));
        assert_eq!(inv.get(11), ItemStack::new(264, 64));
        assert_eq!(inv.cursor, ItemStack::new(264, 56));
        // 未结算时收到 Pickup(0) 点击 → 先回滚 targets 到快照，再执行本次点击
        assert!(inv.apply_click(20, 0, CLICK_PICKUP));
        assert_eq!(
            inv.get(10),
            ItemStack::new(264, 60),
            "取消后槽 10 回滚到拖拽前"
        );
        assert_eq!(
            inv.get(11),
            ItemStack::new(264, 60),
            "取消后槽 11 回滚到拖拽前"
        );
        assert_eq!(
            inv.get(20),
            ItemStack::new(264, 56),
            "取消后 Pickup 与槽 20 石头交换"
        );
        assert_eq!(
            inv.cursor,
            ItemStack::new(1, 3),
            "光标保留拖拽中内容并完成交换"
        );
        assert!(inv.quick_craft.targets.is_empty(), "取消后拖拽状态清除");
    }

    #[test]
    fn quick_craft_cancel_rolls_back_on_drop_cursor() {
        let mut inv = PlayerInventory::new();
        inv.cursor = ItemStack::new(264, 64);
        inv.set(10, ItemStack::new(264, 60));
        inv.set(11, ItemStack::new(264, 60));
        assert!(inv.apply_click(10, 0, CLICK_QUICK_CRAFT));
        assert!(inv.apply_click(11, 1, CLICK_QUICK_CRAFT));
        assert!(inv.drop_cursor());
        assert_eq!(inv.get(10), ItemStack::new(264, 60), "关窗取消：槽 10 回滚");
        assert_eq!(inv.get(11), ItemStack::new(264, 60), "关窗取消：槽 11 回滚");
        assert!(inv.cursor.is_air(), "drop_cursor 清空光标");
        assert!(inv.quick_craft.targets.is_empty(), "关窗取消后拖拽状态清除");
    }

    #[test]
    fn quick_craft_invalid_button_rejected() {
        let mut inv = PlayerInventory::new();
        inv.cursor = ItemStack::new(264, 10);
        assert!(!inv.apply_click(10, 4, CLICK_QUICK_CRAFT));
    }

    // ============ full_sync 标记 ============

    #[test]
    fn apply_click_marks_full_sync_on_change() {
        let mut inv = PlayerInventory::new();
        assert!(inv.full_sync, "new() 时 full_sync 应为 true");
        inv.full_sync = false;
        inv.set(10, ItemStack::new(264, 10));
        assert!(inv.apply_click(10, 0, CLICK_PICKUP));
        assert!(inv.full_sync, "产生状态变化的点击应置位 full_sync");
    }

    #[test]
    fn apply_click_no_change_does_not_mark_full_sync() {
        let mut inv = PlayerInventory::new();
        inv.full_sync = false;
        // 空槽整取无状态变化
        assert!(!inv.apply_click(10, 0, CLICK_PICKUP));
        assert!(!inv.full_sync, "无变化的点击不应置位 full_sync");
    }

    #[test]
    fn quick_craft_clone_mark_full_sync() {
        let mut inv = PlayerInventory::new();
        inv.set_game_mode(GameMode::Creative);
        inv.full_sync = false;
        inv.set(10, ItemStack::new(264, 10));
        assert!(inv.apply_click(10, 0, CLICK_CLONE));
        assert!(inv.full_sync, "克隆改变光标后应置位 full_sync");
    }

    // ============ 不支持的 mode / 越界 ============

    #[test]
    fn apply_clone_rejected_in_survival() {
        let mut inv = PlayerInventory::new(); // 默认生存，克隆被拒
        inv.set(10, ItemStack::new(264, 10));
        assert!(!inv.apply_click(10, 0, CLICK_CLONE));
    }

    #[test]
    fn apply_quick_craft_start_requires_cursor() {
        let mut inv = PlayerInventory::new();
        inv.set(10, ItemStack::new(264, 10));
        // 光标为 AIR → 无物可拖，阶段 0 直接拒绝。
        assert!(!inv.apply_click(10, 0, CLICK_QUICK_CRAFT));
    }

    #[test]
    fn apply_unknown_mode() {
        let mut inv = PlayerInventory::new();
        inv.set(10, ItemStack::new(264, 10));
        assert!(!inv.apply_click(10, 0, 99));
    }

    #[test]
    fn apply_out_of_range_window_slot() {
        let mut inv = PlayerInventory::new();
        inv.set(10, ItemStack::new(264, 10));
        assert!(!inv.apply_click(46, 0, CLICK_PICKUP));
        assert!(!inv.apply_click(-1, 0, CLICK_PICKUP));
    }

    #[test]
    fn apply_window_slot_offhand() {
        let mut inv = PlayerInventory::new();
        inv.set(45, ItemStack::new(264, 10)); // 副手
        assert!(inv.apply_click(45, 0, CLICK_PICKUP));
        assert_eq!(inv.cursor, ItemStack::new(264, 10));
        assert!(inv.get(45).is_air());
    }

    // ============ drop_cursor ============

    #[test]
    fn drop_cursor_non_empty_returns_true_and_clears() {
        let mut inv = PlayerInventory::new();
        inv.cursor = ItemStack::new(264, 5);
        assert!(inv.drop_cursor());
        assert!(inv.cursor.is_air());
    }

    #[test]
    fn drop_cursor_empty_returns_false() {
        let mut inv = PlayerInventory::new();
        assert!(!inv.drop_cursor());
        assert!(inv.cursor.is_air());
    }

    #[test]
    fn drop_cursor_twice_second_false() {
        let mut inv = PlayerInventory::new();
        inv.cursor = ItemStack::new(264, 5);
        assert!(inv.drop_cursor());
        assert!(!inv.drop_cursor());
    }

    // ============ set_held_slot ============

    #[test]
    fn set_held_slot_valid_0_and_8() {
        let mut inv = PlayerInventory::new();
        assert!(inv.set_held_slot(0));
        assert_eq!(inv.held_slot, 0);
        assert!(inv.set_held_slot(8));
        assert_eq!(inv.held_slot, 8);
    }

    #[test]
    fn set_held_slot_invalid_9_and_255() {
        let mut inv = PlayerInventory::new();
        let before = inv.held_slot;
        assert!(!inv.set_held_slot(9));
        assert_eq!(inv.held_slot, before);
        assert!(!inv.set_held_slot(255));
        assert_eq!(inv.held_slot, before);
    }

    // ============ add_item ============

    #[test]
    fn add_item_stacks_to_existing_and_returns_remainder() {
        let mut inv = PlayerInventory::new();
        // 用石头（material=1）填满除槽 10 外的全部 45 槽，使钻石剩余无其他可放之处。
        for s in 0..46usize {
            if s != 10 {
                inv.set(s, ItemStack::new(1, 64));
            }
        }
        inv.set(10, ItemStack::new(264, 50));
        // 堆叠到已有 50 个钻石，受 max_stack 64 限制补满至 64；剩余 6 因其余槽已满（石头，不可堆叠）而返回。
        // 见 `.specs/implement-item-inventory/` R3 场景（满槽截断 + 剩余返回）。
        let remainder = inv.add_item(ItemStack::new(264, 20));
        assert_eq!(inv.get(10), ItemStack::new(264, 64));
        // 满槽截断后剩余 6 个钻石无法放入（其余槽均为满石头），返回原剩余。
        assert_eq!(remainder, ItemStack::new(264, 6));
        assert!(inv.dirty.contains(&10));
    }

    #[test]
    fn add_item_fills_empty_slot() {
        let mut inv = PlayerInventory::new();
        let remainder = inv.add_item(ItemStack::new(264, 10));
        assert!(remainder.is_air());
        // 10 个钻石应进入某空槽并标记脏。
        let placed = (0..46).any(|s| {
            let stack = inv.get(s);
            let ok = stack == ItemStack::new(264, 10);
            ok && inv.dirty.contains(&(s as u8))
        });
        assert!(placed);
    }

    #[test]
    fn add_item_overflow_returns_original() {
        let mut inv = PlayerInventory::new();
        // 用石头（material=1）填满全部 46 槽，无法与钻石堆叠。
        for s in 0..46usize {
            inv.set(s, ItemStack::new(1, 64));
        }
        let input = ItemStack::new(264, 5);
        let remainder = inv.add_item(input.clone());
        assert_eq!(remainder, input);
        assert!((0..46).all(|s| inv.get(s).material != 264));
    }

    #[test]
    fn add_item_air_is_noop() {
        let mut inv = PlayerInventory::new();
        inv.set(10, diamond());
        let remainder = inv.add_item(ItemStack::AIR);
        assert!(remainder.is_air());
        // 库存不变：槽 10 仍持有钻石。
        assert_eq!(inv.get(10), diamond());
    }

    #[test]
    fn add_item_different_components_do_not_stack() {
        let mut inv = PlayerInventory::new();
        let mut pearl = ItemStack::new(368, 10);
        pearl.components.set(ComponentValue::MaxStackSize(16));
        inv.set(10, pearl.clone());
        // 传入无组件的末影珍珠（components 不同），不应堆叠到槽 10。
        let remainder = inv.add_item(ItemStack::new(368, 5));
        assert!(remainder.is_air());
        assert_eq!(inv.get(10), pearl);
        // 5 个无组件末影珍珠落入某空槽。
        assert!((0..46).any(|s| inv.get(s) == ItemStack::new(368, 5)));
    }

    // ============ T10：Inventory trait 实现 ============

    #[test]
    fn inventory_trait_size_get_set_and_dirty() {
        let mut inv = PlayerInventory::new();
        // 46 槽 + get/set 委托既有行为（非 AIR 标脏）。
        assert_eq!(inv.size(), 46);
        inv.set_item(10, ItemStack::new(264, 10));
        assert_eq!(inv.get_item(10), ItemStack::new(264, 10));
        assert!(inv.dirty_slots().contains(&10));
        inv.clear_dirty();
        assert!(inv.dirty_slots().is_empty());
    }

    #[test]
    fn inventory_trait_cursor_delegation() {
        let mut inv = PlayerInventory::new();
        assert!(inv.get_cursor().is_air());
        inv.set_cursor(ItemStack::new(264, 5));
        assert_eq!(inv.get_cursor(), ItemStack::new(264, 5));
        assert_eq!(inv.cursor, ItemStack::new(264, 5));
    }

    #[test]
    fn inventory_trait_out_of_bounds_silent() {
        let mut inv = PlayerInventory::new();
        // 越界写静默忽略（dirty 不含越界槽），越界读返回 AIR。
        inv.set_item(999, diamond());
        assert!(inv.dirty.is_empty());
        assert!(inv.get_item(999).is_air());
    }
}
