// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 类型擦除的组件列存储：SoA 列 + SparseSet 混合，按 `ComponentId` 索引。
//!
//! 变更标识符：`implement-custom-ecs`
//!
//! 每个 Archetype 一个 [`ArchetypeStorage`]：`slots` 是该 Archetype 的实体
//! 列表（与 SoA 列同索引对齐），`columns` 为组件列表。组件列按
//! [`crate::component::Component::STORAGE`] 分为两类（R3）：
//! - **SoA**（[`soa::SoAColumn`]）：与 `slots` 严格对齐的紧凑连续列，热数据
//!   缓存友好（R3.1）；列创建于首次 `insert`（惰性，见 `World::insert`）。
//! - **Sparse**（[`sparse_set::SparseSet`]）：按实体槽位索引，可对任意实体
//!   任意增删，不随 Archetype 搬迁（R3.2/R3.3）。
//!
//! 本目录为 unsafe 白名单（章程「需要局部 unsafe 代码的情况」）：列拆借
//! 方法 [`ErasedColumn::as_any_mut_unchecked`] 供 Query 可变访问（A8），
//! 全部以 `# Safety` + debug_assert 约束。

#![allow(unsafe_code)]

pub mod soa;
pub mod sparse_set;

use std::any::Any;
use std::collections::HashMap;

use crate::archetype::ArchetypeDef;
use crate::component::ComponentId;
use crate::entity::Entity;

/// 类型擦除的组件列：`World` 内按 `ComponentId` 索引，实际存储为
/// `SoAColumn<T>` 或 `SparseSet<T>`，经 `as_any`/`as_any_mut` 下转回具体类型。
///
/// `Send + Sync`：World 需跨线程共享（R11 每 Instance 一个 World，R9
/// 调度器多线程 tick），列随 World 迁移线程，故列类型必须可跨线程发送。
pub trait ErasedColumn: Send + Sync + 'static {
    /// 只读类型擦除（`downcast_ref` 用）。
    fn as_any(&self) -> &dyn Any;
    /// 可变类型擦除（`downcast_mut` 用）。
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// 从共享引用拆借可变访问（Query `fetch_mut` 用，A8）。
    ///
    /// # Safety
    ///
    /// 调用方必须保证该列当前无其他可变借用（包括此前产出的仍存活的
    /// `&mut` 引用）。本实现经 `UnsafeCell::from_ref`（共享→可变的唯一合法
    /// 途径）重建可变引用；调用方（`IterMut`/`get_mut`）由 `World::query_mut`
    /// 独占构造保证互斥。
    ///
    /// `&self → &mut` 为内部 UnsafeCell 拆借（unsafe 契约下仅存于此签名），
    /// 属刻意设计，抑制 `mut_from_ref`。
    #[allow(clippy::mut_from_ref)]
    unsafe fn as_any_mut_unchecked(&self) -> &mut dyn Any;

    /// 是否为按实体槽位索引的 Sparse 存储（不由 Archetype 槽位对齐）。
    fn is_sparse(&self) -> bool;

    /// 删除并返回 Archetype 槽位索引处的元素（SoA：取走该槽并重置为默认值，
    /// 列与 `slots` 保持对齐；Sparse：恒 `None`）。
    fn take_at(&mut self, archetype_index: usize) -> Option<Box<dyn Any + Send + Sync>>;

    /// 按实体槽位删除并返回元素（Sparse：移除该槽值；SoA：恒 `None`）。
    fn take_slot(&mut self, entity_slot: u32) -> Option<Box<dyn Any + Send + Sync>>;

    /// 写入类型擦除值到指定索引（T8 迁移用）。
    ///
    /// `downcast` 还原具体类型后写入；类型不匹配（downcast 失败）或索引越界
    /// 返回 `false`，不产生部分写入。
    ///
    /// - SoA：`index` 为 Archetype 槽位索引（须与 `slots` 对齐，越界视为违反
    ///   对齐不变量，debug 构建下断言暴露）。
    /// - Sparse：`index` 为实体槽位（`Entity::slot().0`），越界自动扩容。
    fn insert_at(&mut self, index: usize, value: Box<dyn Any + Send + Sync>) -> bool;

    /// 新实体入列（`World::spawn`）时追加默认占位：SoA push 默认值保持与
    /// `slots` 对齐；Sparse 按槽索引存储，随实体增长无意义，no-op。
    fn push_default(&mut self);

    /// 实体销毁（`World::despawn`）同步：SoA 列 `swap_remove` 维持与 `slots`
    /// 对齐；Sparse 列移除该实体槽位的值。
    fn on_despawn(&mut self, archetype_index: usize, entity_slot: u32);

    /// 当前元素数（内存统计 / 调试）。
    fn len(&self) -> usize;
    /// 是否无元素。
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// 已分配容量（内存统计，R13.2 数据源）。
    fn capacity(&self) -> usize;
}

/// 单个 Archetype 的运行时存储：实体列表 + 组件列表。
///
/// 字段为 `pub`：供兄弟模块（T3 world / T4 query）直接访问。
pub struct ArchetypeStorage {
    /// 该存储对应的静态 Archetype 定义（拥有副本：Clone/Copy，便于运行时
    /// 合成空 Archetype，T11 迁移；引用字段仍为 `'static`，无额外生命周期）。
    pub def: ArchetypeDef,
    /// 实体列表，与 SoA 列同索引对齐；槽位索引即列索引。
    pub slots: Vec<Entity>,
    /// 组件列表：SoA（与 slots 对齐）与 Sparse（按实体槽位索引）混合。
    pub columns: HashMap<ComponentId, Box<dyn ErasedColumn>>,
}
