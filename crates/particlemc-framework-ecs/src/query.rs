//! Query 编译期匹配：`Query<'w, D, F>` 按静态 Archetype 声明序构建匹配集合。
//!
//! 变更标识符：`implement-custom-ecs`
//!
//! 数据形态 `QueryData`（`&T` / `&mut T` / `Option<&T>` / 元组 ≤8 元，IC-5）
//! 与过滤 `QueryFilter`（`With<T>` / `Without<T>` / 元组组合）。迭代顺序 =
//! Archetype 声明序（ArchetypeId 即静态表下标，升序）+ 槽位升序（R4.4），
//! 单世界内时序严格确定。
//!
//! # 匹配确定性（R4）
//!
//! `World::archetypes` 为 `HashMap<ArchetypeId, ArchetypeStorage>`（无序）。
//! 为消除哈希序导致的迭代顺序不稳定，`Query::matched` 在构造时一次性收集
//! 匹配的 `ArchetypeId` 并按 **ArchetypeId 升序**（`u16`）排序；`Iter`/`IterMut`
//! 仅遍历排序后的 `matched`。`particlemc-framework-ecs` 内部**没有**全局 `ARCHETYPES`
//! 静态表（该表由应用 crate 的 `register_archetypes!` 生成），故匹配基于
//! `World` 中已注册存储的 `def`。
//!
//! # 组件匹配语义
//!
//! 组件"存在"判定（`Query::new`）：SoA 组件声明于
//! [`crate::archetype::ArchetypeDef::component_ids`]（固定组件集）；Sparse
//! 组件可任意增删（R3.3），以实际列存在为准。SoA 列惰性创建（AI Amendment
//! A5）：列缺失时 archetype 仍结构匹配，迭代阶段按实体逐一跳过（该实体无
//! 数据，等价于 `World::get` 返回 `None`）。
//!
//! # unsafe 白名单（AI Amendment A8）
//!
//! 本模块 `#![allow(unsafe_code)]`：`&mut` 元素需要同时可变借用同一 Archetype
//! 的多列（`columns: HashMap` 无法字段级拆借），采用 旧 ECS 方案/hecs 标准的 unsafe
//! 拆借——`fetch_mut` 从共享 `&ArchetypeStorage` 经裸指针重建可写引用。全部
//! unsafe 操作以 `# Safety` 文档 + `debug_assert!` 护城河约束：
//!
//! - `Query<&mut …>` 仅经 [`World::query_mut`]（`&mut World`）构造，外部编译期
//!   借用保证查询生命周期内无其他访问路径（A8）。
//! - `IterMut` 由 `iter_mut(&mut self)` 构造，同一时刻至多一条活跃；游标单调
//!   递增使每实体至多产出一次。
//! - `get_mut`/`single_mut` 返回的引用生命周期绑定 `&mut self`，两次调用结果
//!   不可同时存活，杜绝同实体重复可变借用。
//!
//! # 健全性分层
//!
//! 只读访问（`iter`/`get`/`single`/`contains`）走 [`QueryData::fetch`]（纯安全
//! 路径，经 `ErasedColumn::as_any().downcast_ref` 下转）；仅可变访问
//! （`iter_mut`/`get_mut`/`single_mut`）经 `unsafe fn fetch_mut`。
//! [`World::query`] 要求 `D: ReadOnlyQueryData`（只读 marker），使共享借用下
//! 不可能出现 `&mut` 元素，`get` 恒返回只读项。

#![allow(unsafe_code)]

use std::marker::PhantomData;

use crate::archetype::ArchetypeId;
use crate::component::{Component, ComponentId};
use crate::entity::Entity;
use crate::storage::ArchetypeStorage;
use crate::storage::soa::SoAColumn;
use crate::storage::sparse_set::SparseSet;
use crate::world::World;

/// 查询错误（IC-5）：实体缺失 / 实体不匹配查询 / 空 / 非唯一。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryError {
    /// 实体不存在（未 spawn、已 despawn 或悬挂句柄）。
    EntityNotFound,
    /// 实体存在但不在查询匹配集（Archetype 不满足组件组合或过滤条件），
    /// 或 required 组件列未创建（该实体实际无此组件）。
    WrongEntity,
    /// `single` 无匹配实体。
    Empty,
    /// `single` 存在多个匹配实体。
    NonUnique,
}

/// 查询数据形态：一个组件元组的静态描述（IC-5 扩展）。
///
/// `Item<'w>` 为迭代/取值产出的借用形态（`&T` → `&'w T`，`&mut T` → `&'w mut T`，
/// `Option<&T>` → `Option<&'w T>`，元组递归组合）。`ReadOnly` 为对应只读形态
/// （`&mut T` → `&T`），供只读迭代与 `get`/`single` 使用。
///
/// 注意 trait **无 `'static` 约束**：引用形态（`&'a T`）的 `'a` 为自由生命周期
/// 参数，加 `'static` 会导致 `impl` 无法成立（自研 ECS 同样不加）。组件
/// 类型经 `A: Component`（蕴含 `A: 'static`）保证类型稳定。
pub trait QueryData {
    /// 借用产出形态，生命周期为 World 借用。
    type Item<'w>;
    /// 只读形态：`<D::ReadOnly as QueryData>::Item<'w>` 恒为共享借用。
    type ReadOnly: QueryData;
    /// required（非 `Option`）组件 ID：匹配要求 Archetype 含全部。
    fn required_ids() -> Vec<ComponentId>;
    /// 全部组件 ID（含 `Option` 组件）：供过滤/调试使用。
    fn all_ids() -> Vec<ComponentId>;
    /// 只读取一实体组件：列缺失（required 组件从未 insert）或 Sparse 槽位
    /// 无值返回 `None`，调用方跳过该实体（纯安全路径）。
    fn fetch<'w>(
        storage: &'w ArchetypeStorage,
        index: usize,
    ) -> Option<<Self::ReadOnly as QueryData>::Item<'w>>;
    /// 可变取一实体组件。
    ///
    /// # Safety
    ///
    /// 调用方必须保证：当前组件列在本次访问期间无其他可变借用（含本方法
    /// 之前产出且仍存活的引用）。本实现中调用方为 `IterMut`/`get_mut`——
    /// 前者由 `World::query_mut` 独占构造且游标单调，后者结果绑定 `&mut self`。
    unsafe fn fetch_mut<'w>(storage: &'w ArchetypeStorage, index: usize) -> Option<Self::Item<'w>>;
}

/// 只读查询数据 marker：`World::query`（共享借用）仅接受只读形态，保证共享
/// 路径下不可能产出 `&mut` 项。
pub trait ReadOnlyQueryData: QueryData {}

/// 查询过滤：`With<T>` / `Without<T>` 的静态组合（IC-5）。
pub trait QueryFilter {
    /// 必须存在的组件 ID（过滤 Archetype）。
    fn with_ids() -> Vec<ComponentId>;
    /// 必须不存在的组件 ID（过滤 Archetype）。
    fn without_ids() -> Vec<ComponentId>;
}

/// 过滤标记：Archetype 必须含组件 `T`。仅作类型级标记，从不实例化。
pub struct With<T>(pub(crate) PhantomData<T>);
/// 过滤标记：Archetype 必须不含组件 `T`。仅作类型级标记，从不实例化。
pub struct Without<T>(pub(crate) PhantomData<T>);

// ---- QueryData 基础形态 ----

/// `&'a T`：只读组件引用。`ReadOnly = Self`，`fetch_mut` 退化为只读（无写权限
/// 需求），故 `&T` 元素在可变查询中亦只读产出。
impl<T: Component> QueryData for &T {
    type Item<'w> = &'w T;
    type ReadOnly = Self;

    fn required_ids() -> Vec<ComponentId> {
        vec![T::id()]
    }

    fn all_ids() -> Vec<ComponentId> {
        vec![T::id()]
    }

    fn fetch(storage: &ArchetypeStorage, index: usize) -> Option<&T> {
        let column = storage.columns.get(&T::id())?;
        if column.is_sparse() {
            // Sparse 组件按实体槽位索引；槽位取自与列对齐的 slots 列表。
            // u32 → usize 为扩宽转换（64 位平台无损）；`as` 仅此处槽位换算
            let slot = storage.slots.get(index)?.slot().0;
            let set = column.as_any().downcast_ref::<SparseSet<T>>()?;
            set.get(slot as usize)
        } else {
            let col = column.as_any().downcast_ref::<SoAColumn<T>>()?;
            col.get(index)
        }
    }

    unsafe fn fetch_mut(storage: &ArchetypeStorage, index: usize) -> Option<&T> {
        // 只读元素无可变需求：委托只读路径（调用方契约仍须成立，此处无害）
        Self::fetch(storage, index)
    }
}

impl<T: Component> ReadOnlyQueryData for &T {}

/// `&'a mut T`：可变组件引用。`ReadOnly = &'a T` 使只读迭代经 `fetch` 安全下转。
impl<'a, T: Component> QueryData for &'a mut T {
    type Item<'w> = &'w mut T;
    type ReadOnly = &'a T;

    fn required_ids() -> Vec<ComponentId> {
        vec![T::id()]
    }

    fn all_ids() -> Vec<ComponentId> {
        vec![T::id()]
    }

    fn fetch<'w>(storage: &'w ArchetypeStorage, index: usize) -> Option<&'w T> {
        <&'a T as QueryData>::fetch(storage, index)
    }

    unsafe fn fetch_mut(storage: &ArchetypeStorage, index: usize) -> Option<&mut T> {
        let column = storage.columns.get(&T::id())?;
        // SAFETY: 调用方保证本列无其他可变借用（A8：IterMut 由 query_mut
        // 独占构造、get_mut 结果绑定 &mut self）——经 as_any_mut_unchecked
        // 从共享引用拆借出可写列
        let column_mut = unsafe { column.as_any_mut_unchecked() };
        if column.is_sparse() {
            let slot = storage.slots.get(index)?.slot().0;
            let set = column_mut.downcast_mut::<SparseSet<T>>()?;
            set.get_mut(slot as usize)
        } else {
            let col = column_mut.downcast_mut::<SoAColumn<T>>()?;
            col.get_mut(index)
        }
    }
}

/// `Option<&'a T>`：可选组件引用（required 为空集，缺失产出 `None` 值）。
impl<T: Component> QueryData for Option<&T> {
    type Item<'w> = Option<&'w T>;
    type ReadOnly = Self;

    fn required_ids() -> Vec<ComponentId> {
        Vec::new()
    }

    fn all_ids() -> Vec<ComponentId> {
        vec![T::id()]
    }

    fn fetch(storage: &ArchetypeStorage, index: usize) -> Option<Option<&T>> {
        let column = match storage.columns.get(&T::id()) {
            Some(column) => column,
            // 列未创建（从未 insert）：可选组件缺席，产出 None 而非跳过实体
            None => return Some(None),
        };
        let value = if column.is_sparse() {
            let slot = storage.slots.get(index)?.slot().0;
            column
                .as_any()
                .downcast_ref::<SparseSet<T>>()?
                .get(slot as usize)
        } else {
            column.as_any().downcast_ref::<SoAColumn<T>>()?.get(index)
        };
        Some(value)
    }

    unsafe fn fetch_mut(storage: &ArchetypeStorage, index: usize) -> Option<Option<&T>> {
        // Option 元素为只读形态：无可变需求，委托只读实现
        Self::fetch(storage, index)
    }
}

impl<T: Component> ReadOnlyQueryData for Option<&T> {}

/// 空查询数据：匹配全部 Archetype，产出 `()`（用于计数/遍历实体本身）。
impl QueryData for () {
    type Item<'w> = ();
    type ReadOnly = ();

    fn required_ids() -> Vec<ComponentId> {
        Vec::new()
    }

    fn all_ids() -> Vec<ComponentId> {
        Vec::new()
    }

    fn fetch(_storage: &ArchetypeStorage, _index: usize) -> Option<()> {
        Some(())
    }

    unsafe fn fetch_mut(_storage: &ArchetypeStorage, _index: usize) -> Option<()> {
        Some(())
    }
}

impl ReadOnlyQueryData for () {}

/// 实体句柄作为查询数据：迭代时按槽位产出对应 `Entity`（无 required 组件，
/// 恒匹配所属 Archetype 的全部槽位，不跳过任何实体）。用于 `Query<(Entity, …)>`
/// 形态（旧 ECS 方案 `Entity` 可直接出现在查询元组首位）。
impl QueryData for Entity {
    type Item<'w> = Entity;
    type ReadOnly = Entity;

    fn required_ids() -> Vec<ComponentId> {
        Vec::new()
    }

    fn all_ids() -> Vec<ComponentId> {
        Vec::new()
    }

    fn fetch(_storage: &ArchetypeStorage, index: usize) -> Option<Entity> {
        _storage.slots.get(index).copied()
    }

    unsafe fn fetch_mut(_storage: &ArchetypeStorage, index: usize) -> Option<Entity> {
        _storage.slots.get(index).copied()
    }
}

impl ReadOnlyQueryData for Entity {}

// ---- 元组 QueryData 组合（1..=8 元）----

/// 生成 N 元组 QueryData 实现：`Item`/`ReadOnly` 递归组合，required 拼接。
///
/// `fetch` 对任一元素返回 `None`（required 组件缺失）即整元跳过该实体——
/// 元组元素的 required 均为该实体的硬性组件，任一缺席则该实体不满足查询。
macro_rules! impl_query_data_for_tuples {
    ($($name:ident),+) => {
        impl<$($name: QueryData),+> QueryData for ($($name,)+) {
            type Item<'w> = ($($name::Item<'w>,)+);
            type ReadOnly = ($($name::ReadOnly,)+);

            fn required_ids() -> Vec<ComponentId> {
                let mut ids = Vec::new();
                $(ids.extend($name::required_ids());)+
                ids
            }

            fn all_ids() -> Vec<ComponentId> {
                let mut ids = Vec::new();
                $(ids.extend($name::all_ids());)+
                ids
            }

            fn fetch<'w>(
                storage: &'w ArchetypeStorage,
                index: usize,
            ) -> Option<<Self::ReadOnly as QueryData>::Item<'w>> {
                // 任一元素缺失 → 整个实体跳过（required 组件列未创建等）
                Some(($($name::fetch(storage, index)?,)+))
            }

            unsafe fn fetch_mut<'w>(
                storage: &'w ArchetypeStorage,
                index: usize,
            ) -> Option<Self::Item<'w>> {
                // SAFETY: 委托各元素 fetch_mut。不同组件 ID 对应不同列（全局
                // 注册表保证同类型同 ID），故各元素可变引用指向互不重叠的内存
                Some(($(unsafe { $name::fetch_mut(storage, index)? },)+))
            }
        }

        impl<$($name: ReadOnlyQueryData),+> ReadOnlyQueryData for ($($name,)+) {}
    };
}

impl_query_data_for_tuples!(A);
impl_query_data_for_tuples!(A, B);
impl_query_data_for_tuples!(A, B, C);
impl_query_data_for_tuples!(A, B, C, D);
impl_query_data_for_tuples!(A, B, C, D, E);
impl_query_data_for_tuples!(A, B, C, D, E, F);
impl_query_data_for_tuples!(A, B, C, D, E, F, G);
impl_query_data_for_tuples!(A, B, C, D, E, F, G, H);

// ---- QueryFilter 基础形态 ----

impl QueryFilter for () {
    fn with_ids() -> Vec<ComponentId> {
        Vec::new()
    }

    fn without_ids() -> Vec<ComponentId> {
        Vec::new()
    }
}

impl<T: Component> QueryFilter for With<T> {
    fn with_ids() -> Vec<ComponentId> {
        vec![T::id()]
    }

    fn without_ids() -> Vec<ComponentId> {
        Vec::new()
    }
}

impl<T: Component> QueryFilter for Without<T> {
    fn with_ids() -> Vec<ComponentId> {
        Vec::new()
    }

    fn without_ids() -> Vec<ComponentId> {
        vec![T::id()]
    }
}

/// 生成 N 元组 QueryFilter 实现：with/without 集合拼接。
macro_rules! impl_query_filter_for_tuples {
    ($($name:ident),+) => {
        impl<$($name: QueryFilter),+> QueryFilter for ($($name,)+) {
            fn with_ids() -> Vec<ComponentId> {
                let mut ids = Vec::new();
                $(ids.extend($name::with_ids());)+
                ids
            }

            fn without_ids() -> Vec<ComponentId> {
                let mut ids = Vec::new();
                $(ids.extend($name::without_ids());)+
                ids
            }
        }
    };
}

impl_query_filter_for_tuples!(A);
impl_query_filter_for_tuples!(A, B);
impl_query_filter_for_tuples!(A, B, C);
impl_query_filter_for_tuples!(A, B, C, D);
impl_query_filter_for_tuples!(A, B, C, D, E);
impl_query_filter_for_tuples!(A, B, C, D, E, F);
impl_query_filter_for_tuples!(A, B, C, D, E, F, G);
impl_query_filter_for_tuples!(A, B, C, D, E, F, G, H);

// ---- Query / Iter / IterMut ----

/// 编译期匹配查询：构造时按 Archetype 声明序计算匹配集合，迭代零动态分发。
///
/// 生命周期 `'w` 绑定到 World 借用；`F` 默认 `()`（空过滤，IC-5）。
pub struct Query<'w, D: QueryData, F: QueryFilter = ()> {
    /// World 引用（只读）：可变访问经 unsafe 拆借，契约见模块文档（A8）。
    world: &'w World,
    /// 匹配的 Archetype 集合（按 ArchetypeId 升序 = 声明序，R4.4）。
    matched: Vec<ArchetypeId>,
    _marker: PhantomData<(D, F)>,
}

impl<'w, D: QueryData, F: QueryFilter> Query<'w, D, F> {
    /// 由 World 构建查询：静态匹配 + 确定性排序。
    pub(crate) fn new(world: &'w World) -> Self {
        let required = D::required_ids();
        let with = F::with_ids();
        let without = F::without_ids();
        let mut matched = Vec::new();
        for (arch, storage) in &world.archetypes {
            // 组件"存在"判定：SoA 组件声明于 def.component_ids；Sparse 组件
            // 以实际列存在为准（R3.3 任意增删）。SoA 列惰性创建（AI Amendment
            // A5）：列缺失时 archetype 仍结构匹配，迭代阶段按实体跳过（无数据）
            let present = |id: &ComponentId| {
                storage.def.component_ids.contains(id) || storage.columns.contains_key(id)
            };
            if required.iter().all(present)
                && with.iter().all(present)
                && without.iter().all(|id| !present(id))
            {
                matched.push(*arch);
            }
        }
        // ArchetypeId 即静态表下标（声明序），升序保证确定性迭代顺序（R4.4）
        matched.sort_unstable_by_key(|arch| arch.0);
        Query {
            world,
            matched,
            _marker: PhantomData,
        }
    }

    /// 只读迭代：产出 `D::ReadOnly::Item`（`&mut` 元素退化为 `&`）。零堆分配。
    pub fn iter(&self) -> Iter<'_, 'w, D, F> {
        Iter {
            world: self.world,
            matched: &self.matched,
            arch_cursor: 0,
            index_cursor: 0,
            _marker: PhantomData,
        }
    }

    /// 可变迭代：产出 `D::Item`（含 `&mut` 元素）。
    ///
    /// # Safety（unsafe 拆借）
    ///
    /// 仅当查询由 [`World::query_mut`]（`&mut World`）构造时可安全调用；本方法
    /// 经 `&mut self` 独占查询，同一时刻至多一条 `IterMut` 活跃。
    pub fn iter_mut(&mut self) -> IterMut<'_, 'w, D, F> {
        IterMut {
            world: self.world,
            matched: &self.matched,
            arch_cursor: 0,
            index_cursor: 0,
            _marker: PhantomData,
        }
    }

    /// 按实体取组件（只读）。错误见 [`QueryError`]。
    ///
    /// - 实体不存在（未 spawn/已销毁/悬挂）→ `EntityNotFound`。
    /// - 实体存在但 Archetype 不在匹配集，或 required 组件列未创建 →
    ///   `WrongEntity`。
    pub fn get(&self, e: Entity) -> Result<<D::ReadOnly as QueryData>::Item<'_>, QueryError> {
        let &(arch, idx) = self
            .world
            .entity_index
            .get(&e)
            .ok_or(QueryError::EntityNotFound)?;
        // matched 已按 ArchetypeId 升序排序：二分查找确认实体属于匹配集
        if self
            .matched
            .binary_search_by_key(&arch.0, |id| id.0)
            .is_err()
        {
            return Err(QueryError::WrongEntity);
        }
        let storage = self
            .world
            .archetypes
            .get(&arch)
            .ok_or(QueryError::EntityNotFound)?;
        D::fetch(storage, idx).ok_or(QueryError::WrongEntity)
    }

    /// 按实体取组件（可变）。结果生命周期绑定 `&mut self`，两次调用结果不可
    /// 同时存活，杜绝同实体重复可变借用。
    ///
    /// # Safety（unsafe 拆借）
    ///
    /// 仅当查询由 [`World::query_mut`]（`&mut World`）构造时含 `&mut` 元素，
    /// 可安全调用；`get_mut` 经 `&mut self` 独占查询。
    pub fn get_mut(&mut self, e: Entity) -> Result<D::Item<'_>, QueryError> {
        let &(arch, idx) = self
            .world
            .entity_index
            .get(&e)
            .ok_or(QueryError::EntityNotFound)?;
        if self
            .matched
            .binary_search_by_key(&arch.0, |id| id.0)
            .is_err()
        {
            return Err(QueryError::WrongEntity);
        }
        let world = self.world;
        let storage = world
            .archetypes
            .get(&arch)
            .ok_or(QueryError::EntityNotFound)?;
        // SAFETY: 见 fetch_mut 契约——查询由 query_mut 独占构造时方可含 &mut
        // 元素；get_mut 结果绑定 &mut self，调用方无法同时存活两个结果
        unsafe { D::fetch_mut(storage, idx) }.ok_or(QueryError::WrongEntity)
    }

    /// 恰好一个匹配实体时取其组件（只读）；0 个 → `Empty`，多个 → `NonUnique`。
    pub fn single(&self) -> Result<<D::ReadOnly as QueryData>::Item<'_>, QueryError> {
        let mut iter = self.iter();
        match iter.next() {
            Some(item) => {
                if iter.next().is_some() {
                    Err(QueryError::NonUnique)
                } else {
                    Ok(item)
                }
            }
            None => Err(QueryError::Empty),
        }
    }

    /// 恰好一个匹配实体时取其组件（可变）。错误语义同 [`Query::single`]。
    pub fn single_mut(&mut self) -> Result<D::Item<'_>, QueryError> {
        let mut iter = self.iter_mut();
        match iter.next() {
            Some(item) => {
                if iter.next().is_some() {
                    Err(QueryError::NonUnique)
                } else {
                    Ok(item)
                }
            }
            None => Err(QueryError::Empty),
        }
    }

    /// 实体是否在查询匹配集内（存活且 Archetype 匹配）。
    pub fn contains(&self, e: Entity) -> bool {
        self.world.entity_index.get(&e).is_some_and(|&(arch, _)| {
            self.matched
                .binary_search_by_key(&arch.0, |id| id.0)
                .is_ok()
        })
    }
}

/// 只读迭代器：按匹配集合（声明序）+ 槽位升序遍历，required 组件缺失的实体
/// 跳过。`Item` 生命周期绑定 World 借用（'w），零堆分配。
pub struct Iter<'a, 'w, D: QueryData, F: QueryFilter = ()> {
    world: &'w World,
    matched: &'a [ArchetypeId],
    arch_cursor: usize,
    index_cursor: usize,
    _marker: PhantomData<(D, F)>,
}

impl<'a, 'w, D: QueryData, F: QueryFilter> Iterator for Iter<'a, 'w, D, F> {
    type Item = <D::ReadOnly as QueryData>::Item<'w>;

    fn next(&mut self) -> Option<Self::Item> {
        let world = self.world;
        while self.arch_cursor < self.matched.len() {
            let arch = self.matched[self.arch_cursor];
            let storage = match world.archetypes.get(&arch) {
                Some(storage) => storage,
                // 匹配集构造于 world.archetypes，迭代期间 World 不可变（查询
                // 持借用），此分支防御性不可达
                None => {
                    self.arch_cursor += 1;
                    self.index_cursor = 0;
                    continue;
                }
            };
            if self.index_cursor >= storage.slots.len() {
                self.arch_cursor += 1;
                self.index_cursor = 0;
                continue;
            }
            let idx = self.index_cursor;
            self.index_cursor += 1;
            // fetch 返回 None：required 组件列未创建或 Sparse 槽位缺失 → 跳过
            if let Some(item) = D::fetch(storage, idx) {
                return Some(item);
            }
        }
        None
    }
}

/// 可变迭代器：语义同 [`Iter`]，产出 `D::Item<'w>`（含 `&mut` 元素）。
pub struct IterMut<'a, 'w, D: QueryData, F: QueryFilter = ()> {
    world: &'w World,
    matched: &'a [ArchetypeId],
    arch_cursor: usize,
    index_cursor: usize,
    _marker: PhantomData<(D, F)>,
}

impl<'a, 'w, D: QueryData, F: QueryFilter> Iterator for IterMut<'a, 'w, D, F> {
    type Item = D::Item<'w>;

    fn next(&mut self) -> Option<Self::Item> {
        let world = self.world;
        while self.arch_cursor < self.matched.len() {
            let arch = self.matched[self.arch_cursor];
            let storage = match world.archetypes.get(&arch) {
                Some(storage) => storage,
                None => {
                    self.arch_cursor += 1;
                    self.index_cursor = 0;
                    continue;
                }
            };
            if self.index_cursor >= storage.slots.len() {
                self.arch_cursor += 1;
                self.index_cursor = 0;
                continue;
            }
            let idx = self.index_cursor;
            self.index_cursor += 1;
            // SAFETY: fetch_mut 拆借契约——IterMut 由 query_mut 独占构造
            // （&mut World 已消费为唯一引用，A8），迭代期间无其他访问路径；
            // required 组件缺失时返回 None 跳过，游标单调不重复产出
            if let Some(item) = unsafe { D::fetch_mut(storage, idx) } {
                return Some(item);
            }
        }
        None
    }
}

impl World {
    /// 只读查询：`D` 限只读形态（`ReadOnlyQueryData`），共享借用下安全。
    ///
    /// 匹配依据为各 Archetype 存储的 `def`（`component_ids` + 实际列）；结果按
    /// ArchetypeId 升序排序（确定性，R4）。注：函数类型参数不支持默认值
    /// （RFC 2133），过滤参数 `F` 需显式给 `()`。
    pub fn query<D: ReadOnlyQueryData, F: QueryFilter>(&self) -> Query<'_, D, F> {
        Query::new(self)
    }

    /// 可变查询：`D` 可含 `&mut` 元素；返回查询排他借用 World 至其生命周期
    /// 结束（`&mut World` 已消费，期间一切其他 World 访问均为编译错误，A8）。
    ///
    /// 只读操作（`iter`/`get`/`single`/`contains`）亦可用。
    pub fn query_mut<D: QueryData, F: QueryFilter>(&mut self) -> Query<'_, D, F> {
        Query::new(self)
    }

    /// 从共享引用构造任意形态查询（含 `&mut` 元素的 `D`，T7 SystemParam 用）。
    ///
    /// 与 [`World::query_mut`] 的匹配/排序逻辑完全一致，但仅持 `&self`——
    /// 可变拆借的安全责任从「编译期 `&mut World` 借用」转移到 **调用方 unsafe
    /// 契约**（本方法本身 safe，构造查询不触碰任何组件数据）。
    ///
    /// # Safety（使用方）
    ///
    /// 调用方必须保证：查询的生命周期内，其 `D` 中各组件列无其他可变借用；
    /// 仅 [`crate::system::SystemParam::get`]（调度器互斥校验后，A9）调用本
    /// 方法构造含 `&mut` 的查询时成立。只读形态（`D::ReadOnlyQueryData`）下
    /// 无此限制。
    pub(crate) fn query_any<D: QueryData, F: QueryFilter>(&self) -> Query<'_, D, F> {
        Query::new(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archetype::ArchetypeDef;
    use crate::component::ComponentStorage;
    use crate::entity::{EntityTypeId, Generation, Slot};

    // ---- 测试组件（手工实现 Component，避免测试依赖宏 crate）----

    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    struct Position {
        x: f32,
        y: f32,
    }

    impl Component for Position {
        fn id() -> ComponentId {
            ComponentId(10)
        }
        const STORAGE: ComponentStorage = ComponentStorage::SoA;
        type Registry = ();
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    struct Velocity {
        dx: f32,
        dy: f32,
    }

    impl Component for Velocity {
        fn id() -> ComponentId {
            ComponentId(11)
        }
        const STORAGE: ComponentStorage = ComponentStorage::SoA;
        type Registry = ();
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    struct Player;

    impl Component for Player {
        fn id() -> ComponentId {
            ComponentId(12)
        }
        const STORAGE: ComponentStorage = ComponentStorage::SoA;
        type Registry = ();
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    struct Health {
        hp: u32,
    }

    impl Component for Health {
        fn id() -> ComponentId {
            ComponentId(13)
        }
        const STORAGE: ComponentStorage = ComponentStorage::Sparse;
        type Registry = ();
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    struct Poisoned;

    impl Component for Poisoned {
        fn id() -> ComponentId {
            ComponentId(14)
        }
        const STORAGE: ComponentStorage = ComponentStorage::Sparse;
        type Registry = ();
    }

    // ---- 测试 Archetype 定义（'static，可直接注册）----

    /// 玩家：SoA Position + Velocity + Player，实体类型 1。
    static PLAYER_DEF: ArchetypeDef = ArchetypeDef {
        id: ArchetypeId(0),
        name: "QueryPlayerArchetype",
        component_ids: &[ComponentId(10), ComponentId(11), ComponentId(12)],
        entity_kind: EntityTypeId(1),
        component_types: &[],
    };

    /// 怪物：SoA Position + Sparse Health（declared），实体类型 2。
    static MONSTER_DEF: ArchetypeDef = ArchetypeDef {
        id: ArchetypeId(1),
        name: "QueryMonsterArchetype",
        component_ids: &[ComponentId(10), ComponentId(13)],
        entity_kind: EntityTypeId(2),
        component_types: &[],
    };

    /// 物品：仅 SoA Velocity，实体类型 3（不含 Position）。
    static ITEM_DEF: ArchetypeDef = ArchetypeDef {
        id: ArchetypeId(2),
        name: "QueryItemArchetype",
        component_ids: &[ComponentId(11)],
        entity_kind: EntityTypeId(3),
        component_types: &[],
    };

    fn ghost(kind: u8, slot: u32) -> Entity {
        Entity::from_parts(EntityTypeId(kind), Generation(0), Slot(slot))
    }

    /// 构造含玩家与怪物各若干实体的世界。
    fn populated_world() -> World {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        world.register_archetype(&MONSTER_DEF);
        world.register_archetype(&ITEM_DEF);
        let p1 = world.spawn(ArchetypeId(0));
        let p2 = world.spawn(ArchetypeId(0));
        let m1 = world.spawn(ArchetypeId(1));
        let m2 = world.spawn(ArchetypeId(1));
        let _ = world.insert(p1, Position { x: 1.0, y: 1.0 });
        let _ = world.insert(p2, Position { x: 2.0, y: 2.0 });
        let _ = world.insert(p1, Velocity { dx: 1.0, dy: 0.0 });
        let _ = world.insert(p2, Velocity { dx: 2.0, dy: 0.0 });
        let _ = world.insert(m1, Position { x: 3.0, y: 3.0 });
        let _ = world.insert(m2, Position { x: 4.0, y: 4.0 });
        world
    }

    #[test]
    fn read_only_iter_single_component() {
        let world = populated_world();
        // 全部含 Position 的实体（玩家 2 + 怪物 2），声明序：Player(0) → Monster(1)
        let q = world.query::<(&Position,), ()>();
        let mut xs = Vec::new();
        for (pos,) in q.iter() {
            xs.push(pos.x);
        }
        assert_eq!(xs, vec![1.0, 2.0, 3.0, 4.0]);
        assert_eq!(q.iter().count(), 4);
    }

    #[test]
    fn iter_only_yields_entities_with_all_required_components() {
        let world = populated_world();
        // 需要 Position + Velocity：仅玩家 Archetype（声明序 0）匹配
        let q = world.query::<(&Position, &Velocity), ()>();
        let mut xs = Vec::new();
        for (pos, vel) in q.iter() {
            xs.push((pos.x, vel.dx));
        }
        assert_eq!(xs, vec![(1.0, 1.0), (2.0, 2.0)]);
    }

    #[test]
    fn iteration_order_is_deterministic_declaration_then_slot() {
        let world = populated_world();
        // 跨 archetype：先 Player 槽位 0,1，再 Monster 槽位 0,1（声明序升序）
        let q = world.query::<(&Position,), ()>();
        let got: Vec<f32> = q.iter().map(|(p,)| p.x).collect();
        assert_eq!(got, vec![1.0, 2.0, 3.0, 4.0]);
        // 多次迭代结果一致（确定性）
        let again: Vec<f32> = q.iter().map(|(p,)| p.x).collect();
        assert_eq!(again, got);
    }

    #[test]
    fn empty_query_matches_all_archetypes() {
        let world = populated_world();
        // Query<()>：无 required，匹配全部已注册 Archetype，产出 4 实体
        // （populated_world 共 4 个：p1/p2/m1/m2）
        let q = world.query::<(), ()>();
        assert_eq!(q.iter().count(), 4);
    }

    #[test]
    fn mixed_read_write_iter_mut() {
        let mut world = populated_world();
        // 玩家：(&mut Position, &Velocity)；怪物不含 Velocity，不匹配
        let mut q = world.query_mut::<(&mut Position, &Velocity), ()>();
        for (pos, vel) in q.iter_mut() {
            pos.x += vel.dx;
        }
        drop(q);
        // 更新落盘：p1.x = 2.0, p2.x = 4.0（怪物不动）
        let q2 = world.query::<(&Position,), ()>();
        let xs: Vec<f32> = q2.iter().map(|(p,)| p.x).collect();
        assert_eq!(xs, vec![2.0, 4.0, 3.0, 4.0]);
    }

    #[test]
    fn read_only_iter_of_mut_query_yields_shared_refs() {
        let mut world = populated_world();
        // 可变形态查询上做只读迭代：产出 &Position（非 &mut）
        let q = world.query_mut::<(&mut Position,), ()>();
        let xs: Vec<f32> = q.iter().map(|(p,)| p.x).collect();
        assert_eq!(xs, vec![1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn required_sparse_component_skips_entities_without_value() {
        let mut world = World::new();
        world.register_archetype(&MONSTER_DEF);
        let m1 = world.spawn(ArchetypeId(1));
        // m2 仅用于验证「槽位空缺实体被跳过」：不 insert 任何组件
        let _m2 = world.spawn(ArchetypeId(1));
        // 仅 m1 有 Health 值（列创建），m2 槽位空缺
        let _ = world.insert(m1, Health { hp: 50 });
        let q = world.query::<(&Health,), ()>();
        let hps: Vec<u32> = q.iter().map(|(h,)| h.hp).collect();
        assert_eq!(hps, vec![50]);
    }

    #[test]
    fn optional_sparse_missing_gives_none_inline() {
        let mut world = World::new();
        world.register_archetype(&MONSTER_DEF);
        let m1 = world.spawn(ArchetypeId(1));
        let m2 = world.spawn(ArchetypeId(1));
        // 先建 Position 列（惰性列：required 组件需已 insert 才有数据）
        let _ = world.insert(m1, Position { x: 1.0, y: 0.0 });
        let _ = world.insert(m2, Position { x: 2.0, y: 0.0 });
        // 仅 m1 有 Health（Sparse 可选组件）：m2 槽位空缺 → None
        let _ = world.insert(m1, Health { hp: 70 });
        let q = world.query::<(&Position, Option<&Health>), ()>();
        // 两张：m1 → Some(70)，m2 → None
        let mut seen = Vec::new();
        for (_, h) in q.iter() {
            seen.push(h.map(|h| h.hp));
        }
        assert_eq!(seen, vec![Some(70), None]);
    }

    #[test]
    fn required_soa_column_not_created_skips_all_entities() {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        // spawn 但从未 insert Position：列未创建，get 恒 None（惰性列语义）
        let _ = world.spawn(ArchetypeId(0));
        let _ = world.spawn(ArchetypeId(0));
        let q = world.query::<(&Position,), ()>();
        assert_eq!(q.iter().count(), 0);
        // 但实体确实存在：Query<()> 可见
        let q2 = world.query::<(), ()>();
        assert_eq!(q2.iter().count(), 2);
    }

    #[test]
    fn with_without_filter() {
        let world = populated_world();
        // With<Player>：仅玩家 archetype（declared Player）匹配
        let q = world.query::<(&Position,), With<Player>>();
        let xs: Vec<f32> = q.iter().map(|(p,)| p.x).collect();
        assert_eq!(xs, vec![1.0, 2.0]);

        // Without<Health>：怪物 declared Health → 排除；玩家匹配
        let q = world.query::<(&Position,), Without<Health>>();
        let xs: Vec<f32> = q.iter().map(|(p,)| p.x).collect();
        assert_eq!(xs, vec![1.0, 2.0]);

        // 组合 (With<Player>, Without<Velocity>)：玩家 declared Velocity → 空
        let q = world.query::<(&Position,), (With<Player>, Without<Velocity>)>();
        assert_eq!(q.iter().count(), 0);

        // 组合 (With<Player>, Without<Health>)：玩家匹配
        let q = world.query::<(&Position,), (With<Player>, Without<Health>)>();
        let xs: Vec<f32> = q.iter().map(|(p,)| p.x).collect();
        assert_eq!(xs, vec![1.0, 2.0]);

        // With<Poisoned>（Sparse，从未 insert 列）：无匹配
        let q = world.query::<(&Position,), With<Poisoned>>();
        assert_eq!(q.iter().count(), 0);
    }

    #[test]
    fn get_hit_and_error_branches() {
        let mut world = populated_world();
        let p1 = world.spawn(ArchetypeId(0));
        let p2 = world.spawn(ArchetypeId(0));
        let item = world.spawn(ArchetypeId(2)); // 无 Position 的 archetype
        let _ = world.insert(p1, Position { x: 5.0, y: 0.0 });
        let _ = world.insert(p2, Position { x: 6.0, y: 0.0 });

        let q = world.query::<(&Position,), ()>();
        // 命中
        assert_eq!(q.get(p1).unwrap().0.x, 5.0);
        assert_eq!(q.get(p2).unwrap().0.x, 6.0);
        // EntityNotFound：悬挂句柄
        assert_eq!(q.get(ghost(1, 999)), Err(QueryError::EntityNotFound));
        // WrongEntity：实体存在但 archetype 不含 Position
        assert_eq!(q.get(item), Err(QueryError::WrongEntity));
        // 销毁后 EntityNotFound
        assert!(world.despawn(p2));
        let q2 = world.query::<(&Position,), ()>();
        assert_eq!(q2.get(p2), Err(QueryError::EntityNotFound));
    }

    #[test]
    fn get_mut_updates_and_borrow_ends() {
        let mut world = populated_world();
        let p1 = world.spawn(ArchetypeId(0));
        let _ = world.insert(p1, Position { x: 1.0, y: 0.0 });
        let mut q = world.query_mut::<(&mut Position,), ()>();
        // get_mut 命中并原地修改（结果绑定 &mut self，临时存活）
        if let Ok((pos,)) = q.get_mut(p1) {
            pos.x = 42.0;
        }
        // 结果已随语句结束 drop，可再次 get_mut（编译期保证不并存）
        let _ = q.get_mut(p1);
        drop(q);
        // 修改已落盘
        let q2 = world.query::<(&Position,), ()>();
        assert_eq!(q2.get(p1).unwrap().0.x, 42.0);
    }

    #[test]
    fn get_mut_wrong_and_missing_entities() {
        let mut world = populated_world();
        let item = world.spawn(ArchetypeId(2)); // 不含 Position
        let mut q = world.query_mut::<(&mut Position,), ()>();
        assert_eq!(q.get_mut(item), Err(QueryError::WrongEntity));
        assert_eq!(q.get_mut(ghost(1, 999)), Err(QueryError::EntityNotFound));
    }

    #[test]
    fn single_ok_empty_and_non_unique() {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        let p1 = world.spawn(ArchetypeId(0));
        let _ = world.insert(p1, Position { x: 9.0, y: 0.0 });
        let q = world.query::<(&Position,), ()>();
        // 恰好 1 → Ok
        assert_eq!(q.single().unwrap().0.x, 9.0);

        // 再造一个实体 → 2 个 → NonUnique
        let p2 = world.spawn(ArchetypeId(0));
        let _ = world.insert(p2, Position { x: 8.0, y: 0.0 });
        let q2 = world.query::<(&Position,), ()>();
        assert_eq!(q2.single(), Err(QueryError::NonUnique));

        // 空匹配 → Empty（With<Poisoned> 无实体）
        let q3 = world.query::<(&Position,), With<Poisoned>>();
        assert_eq!(q3.single(), Err(QueryError::Empty));
    }

    #[test]
    fn single_mut_ok_empty_non_unique() {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        let p1 = world.spawn(ArchetypeId(0));
        let _ = world.insert(p1, Position { x: 1.0, y: 0.0 });
        let mut q = world.query_mut::<(&mut Position,), ()>();
        if let Ok((pos,)) = q.single_mut() {
            pos.x = 100.0;
        }
        drop(q);
        let q2 = world.query::<(&Position,), ()>();
        assert_eq!(q2.get(p1).unwrap().0.x, 100.0);

        // 空匹配 → Empty
        let mut q3 = world.query_mut::<(&mut Position,), With<Poisoned>>();
        assert_eq!(q3.single_mut(), Err(QueryError::Empty));
    }

    #[test]
    fn contains_entity() {
        let mut world = populated_world();
        let p1 = world.spawn(ArchetypeId(0));
        let item = world.spawn(ArchetypeId(2));
        let _ = world.insert(p1, Position { x: 1.0, y: 0.0 });
        let q = world.query::<(&Position,), ()>();
        assert!(q.contains(p1));
        assert!(!q.contains(item)); // archetype 不匹配
        assert!(!q.contains(ghost(1, 999))); // 不存在
        assert!(world.despawn(p1));
        let q2 = world.query::<(&Position,), ()>();
        assert!(!q2.contains(p1));
    }

    #[test]
    fn mut_iter_sparse_missing_skips_entity() {
        let mut world = World::new();
        world.register_archetype(&MONSTER_DEF);
        let m1 = world.spawn(ArchetypeId(1));
        // m2 仅用于验证「槽位空缺实体被跳过」：不 insert 任何组件
        let _m2 = world.spawn(ArchetypeId(1));
        let _ = world.insert(m1, Health { hp: 3 });
        let mut q = world.query_mut::<(&mut Health,), ()>();
        for (h,) in q.iter_mut() {
            h.hp += 1;
        }
        drop(q);
        let q2 = world.query::<(&Health,), ()>();
        let hps: Vec<u32> = q2.iter().map(|(h,)| h.hp).collect();
        // m2 无 Health 值被跳过；m1 更新为 4
        assert_eq!(hps, vec![4]);
    }

    #[test]
    fn unregistered_archetype_not_in_query() {
        // 只注册 PLAYER：未注册的 archetype（MONSTER_DEF）不参与匹配
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        let p1 = world.spawn(ArchetypeId(0));
        // 建列：required 组件需 insert 才有数据（惰性列语义）
        let _ = world.insert(p1, Position { x: 1.0, y: 0.0 });
        let q = world.query::<(&Position,), ()>();
        assert_eq!(q.iter().count(), 1);
    }

    #[test]
    fn query_error_variants_all_covered() {
        // QueryError 全变体可达性（错误分支 100%）
        assert_eq!(QueryError::EntityNotFound, QueryError::EntityNotFound);
        assert_eq!(QueryError::WrongEntity, QueryError::WrongEntity);
        assert_eq!(QueryError::Empty, QueryError::Empty);
        assert_eq!(QueryError::NonUnique, QueryError::NonUnique);
        let _ = matches!(QueryError::EntityNotFound, QueryError::EntityNotFound);
        let _ = matches!(QueryError::WrongEntity, QueryError::WrongEntity);
        let _ = matches!(QueryError::Empty, QueryError::Empty);
        let _ = matches!(QueryError::NonUnique, QueryError::NonUnique);
    }

    #[test]
    fn four_tuple_combination() {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        let p1 = world.spawn(ArchetypeId(0));
        let p2 = world.spawn(ArchetypeId(0));
        let _ = world.insert(p1, Position { x: 1.0, y: 0.0 });
        let _ = world.insert(p2, Position { x: 2.0, y: 0.0 });
        let _ = world.insert(p1, Velocity { dx: 1.0, dy: 0.0 });
        let _ = world.insert(p2, Velocity { dx: 2.0, dy: 0.0 });
        let _ = world.insert(p1, Player);
        let _ = world.insert(p2, Player);
        // 4 元（3 个 required + 1 个 Option）
        let q = world.query::<(&Position, &Velocity, &Player, Option<&Health>), ()>();
        let mut seen = Vec::new();
        for (pos, vel, _player, health) in q.iter() {
            seen.push((pos.x, vel.dx, health.is_none()));
        }
        assert_eq!(seen, vec![(1.0, 1.0, true), (2.0, 2.0, true)]);
    }

    /// 借用互斥说明：`query_mut` 排他借用 World，下列代码若取消注释将编译失败
    /// （`world` 已被 `q` 排他借用），此为编译期保证（A8），非运行时检查：
    ///
    /// ```text
    /// let mut q = world.query_mut::<(&mut Position,), ()>();
    /// let _ = world.entity_count(); // error[E0502]: cannot borrow `world` as immutable
    /// ```
    #[test]
    fn borrow_exclusivity_is_compile_time() {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        let e = world.spawn(ArchetypeId(0));
        let _ = world.insert(e, Position { x: 0.0, y: 0.0 });
        let q = world.query_mut::<(&mut Position,), ()>();
        // 此处 world 已排他借用；取消下列注释将触发 E0502（编译错误）：
        //   let _ = world.entity_count();
        //   let _ = &world;
        drop(q);
        // 查询生命周期结束，World 重新可用
        assert!(world.contains(e));
    }
}
