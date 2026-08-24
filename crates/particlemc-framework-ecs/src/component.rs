// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 组件注册：`Component` trait 与惰性全局 `ComponentId` 分配。
//!
//! 变更标识符：`implement-custom-ecs`
//!
//! 模块路径 `particlemc-framework-ecs::component`（IC-2）。`ComponentId` 经
//! `OnceLock<Mutex<Vec<TypeId>>>` 惰性全局分配（AI Amendment A1）：同一类型
//! 恒返回同一 ID，不同类型按注册顺序递增。`#[derive(Component)]`（T2 宏）
//! 生成的 `id()` 会调用 [`register_component_id`]；`STORAGE` 决定组件存储类别
//! （SoA 列 / SparseSet），`Registry` 为宏生成的存储元数据占位关联类型。

use std::any::TypeId;
use std::sync::{Mutex, OnceLock};

/// 组件存储类别。
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ComponentStorage {
    /// 结构体数组（SoA）列存储：同组件紧凑连续、缓存友好（热数据默认，R3.1）。
    SoA,
    /// SparseSet 独立存储：增删 O(1) 且不触发 Archetype 搬迁（冷/高频变动数据，R3.2）。
    Sparse,
}

/// 全局唯一组件标识（惰性分配，IC-2）。
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct ComponentId(pub u16);

/// 组件注册契约：由 `#[derive(Component)]`（T2 宏）生成实现。
///
/// - `id()`：惰性全局分配（启动期 OnceLock 一次性初始化，之后恒定；热路径
///   经缓存查询零开销，见 AI Amendment A1）。
/// - `STORAGE`：存储类别（默认 SoA；`#[component(storage = "sparse")]` 走 Sparse）。
/// - `Registry`：宏生成的存储元数据占位关联类型（列类型/大小/对齐）。
pub trait Component: Sized + 'static {
    /// 惰性全局分配：同一类型恒返回同一 `ComponentId`。
    fn id() -> ComponentId;

    /// 该组件的存储类别。
    const STORAGE: ComponentStorage;

    /// 存储元数据占位关联类型（宏生成）。
    type Registry;
}

/// 全局组件注册表：TypeId → 序号，惰性一次性初始化（启动期恒定）。
///
/// 消费方为 T2 宏生成的 `impl Component`（`#[derive(Component)]` 调用
/// [`register_component_id`]），本任务先行提供冻结接口，故允许暂未使用。
#[allow(dead_code)]
static COMPONENT_REGISTRY: OnceLock<Mutex<Vec<TypeId>>> = OnceLock::new();

/// 惰性注册组件并返回其 `ComponentId`。
///
/// 已注册则返回既有 ID，否则追加到全局表并返回递增序号。`#[derive(Component)]`
/// 宏（particlemc-framework-ecs-macros）生成的 `impl Component` 会调用本函数，宏展开
/// 可能发生在**外部 crate**（如 particlemc-framework-core 中 derive），故必须为 `pub`。
pub fn register_component_id(type_id: TypeId) -> ComponentId {
    let mut table = match COMPONENT_REGISTRY
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
    {
        Ok(guard) => guard,
        // 仅当持锁线程 panic 时锁才会毒化；注册表无中间更新态，恢复后继续使用
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(idx) = table.iter().position(|entry| *entry == type_id) {
        return ComponentId(to_u16(idx));
    }
    table.push(type_id);
    ComponentId(to_u16(table.len() - 1))
}

/// usize 序号安全转 u16：组件总数不可能超过 65536，饱和仅作形式性兜底。
fn to_u16(idx: usize) -> u16 {
    u16::try_from(idx).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TypeA;
    struct TypeB;
    struct TypeC;

    #[test]
    fn component_id_assigned_once_and_incrementing() {
        let a1 = register_component_id(TypeId::of::<TypeA>());
        // 同一 TypeId 恒返回相同 id
        let a2 = register_component_id(TypeId::of::<TypeA>());
        assert_eq!(a1, a2);
        // 不同 TypeId 按注册顺序递增分配
        let b = register_component_id(TypeId::of::<TypeB>());
        let c = register_component_id(TypeId::of::<TypeC>());
        assert_ne!(a1, b);
        assert_ne!(b, c);
        assert!(b.0 > a1.0);
        assert!(c.0 > b.0);
    }
}
