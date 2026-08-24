//! 资源：跨系统共享的单例数据，按类型存储。
//!
//! 变更标识符：`implement-custom-ecs`
//!
//! [`Resource`] 与 旧 ECS 方案 语义一致：任何 `Send + Sync + 'static` 类型均可作为
//! 资源，每类型全局唯一一份。内部 [`ResourceMap`] 按 `TypeId` 索引存储，
//! 支持插入/替换/移除/查询。世界间资源互不可见（R11 实例隔离），只读全局
//! 数据经 `Shared<T>`（T8）注入。
//!
//! # unsafe 白名单（AI Amendment A9）
//!
//! 本模块 `#![allow(unsafe_code)]`：T7a 系统参数要求从共享 `&World` 提取
//! 可变资源（`ResMut`），`HashMap` 无法从 `&self` 安全地给可变引用，故内部
//! 以 [`UnsafeCell`] 承载值，经契约化 [`ResourceMap::get_mut_unchecked`]
//! 完成共享→可变下转。所有可变访问要么经 `&mut self`（编译器独占借用），
//! 要么经调用方契约（系统参数互斥，A9）串行化，`unsafe impl Sync` 在此
//! 前提下成立。

#![allow(unsafe_code)]

use std::any::{Any, TypeId};
use std::cell::UnsafeCell;
use std::collections::HashMap;

/// 跨系统共享的单例资源标记。
///
/// 与 旧 ECS 方案 一致：`T: Send + Sync + 'static` 的任意类型自动实现本 trait。
pub trait Resource: Send + Sync + 'static {}

impl<T: Send + Sync + 'static> Resource for T {}

/// 按 `TypeId` 索引的资源表（crate 内部使用）。
///
/// 值以 [`UnsafeCell`] 包装：`get_mut_unchecked` 从共享引用下转可变引用。
/// `Send` 由结构自动推导（`UnsafeCell<Box<dyn Any + Send + Sync>>` 为 Send）；
/// `Sync` 需手动标记（见下方 SAFETY）。
pub(crate) struct ResourceMap {
    values: HashMap<TypeId, UnsafeCell<Box<dyn Any + Send + Sync>>>,
}

// SAFETY: ResourceMap 的可变访问只有两条路径：(1) 经 `&mut self` 的方法
// （insert/remove/get_mut），由编译器独占借用保证互斥；(2) 契约化 unsafe
// 路径 `get_mut_unchecked`，其调用方（World::resource_mut_unchecked →
// FunctionSystem::run）保证同一系统一次运行内各参数借用互斥（A9），且系统
// 按调度顺序串行执行。只读访问（get/contains）与上述可变访问不会同时存活
// （要么经 `&mut self` 独占、要么经契约串行化），故不存在数据竞争。
// `Send` 为结构自动推导（各字段 Send），此处不再重复声明。
unsafe impl Sync for ResourceMap {}

impl ResourceMap {
    /// 空资源表。
    pub(crate) fn new() -> Self {
        ResourceMap {
            values: HashMap::new(),
        }
    }

    /// 插入资源；同类型已存在时返回被替换的旧值。
    pub(crate) fn insert<T: Resource>(&mut self, r: T) -> Option<T> {
        self.values
            .insert(TypeId::of::<T>(), UnsafeCell::new(Box::new(r)))
            // 键与值同源于 TypeId::of::<T>()，类型必然匹配；.ok() 仅作结构性兜底
            .and_then(|old| old.into_inner().downcast::<T>().ok().map(|boxed| *boxed))
    }

    /// 移除并返回资源；未注册时返回 None。
    pub(crate) fn remove<T: Resource>(&mut self) -> Option<T> {
        self.values
            .remove(&TypeId::of::<T>())
            .and_then(|old| old.into_inner().downcast::<T>().ok().map(|boxed| *boxed))
    }

    /// 只读获取资源引用；未注册时返回 None。
    pub(crate) fn get<T: Resource>(&self) -> Option<&T> {
        let cell = self.values.get(&TypeId::of::<T>())?;
        // SAFETY: 本方法签名 `&self` 及调用方（World::resource）的共享借用
        // 约束保证：返回值存活期内无并存的可变访问；UnsafeCell 内部值在此
        // 仅被共享解引用（不写、不产生可变别名）。
        let inner = unsafe { &*cell.get() };
        inner.downcast_ref::<T>()
    }

    /// 可变获取资源引用；未注册时返回 None。
    pub(crate) fn get_mut<T: Resource>(&mut self) -> Option<&mut T> {
        // UnsafeCell::get_mut 要求独占 `&mut`，由本方法签名 `&mut self` 提供，
        // 编译器保证与一切其他访问互斥，纯安全路径。
        self.values
            .get_mut(&TypeId::of::<T>())
            .map(|cell| cell.get_mut())
            .and_then(|any| any.downcast_mut::<T>())
    }

    /// 从共享引用获取可变资源引用（系统参数提取路径）。
    ///
    /// # Safety
    ///
    /// 调用方必须保证：返回的 `&mut T` 生命周期内，该资源不存在其他可变或
    /// 共享借用（与同系统其他参数互斥，A9 契约）。本 crate 中该义务由
    /// [`crate::system::FunctionSystem::run`] 承担：系统按调度顺序串行执行，
    /// 一次运行内各参数借用互斥。
    #[allow(clippy::mut_from_ref)]
    pub(crate) unsafe fn get_mut_unchecked<T: Resource>(&self) -> Option<&mut T> {
        let cell = self.values.get(&TypeId::of::<T>())?;
        // SAFETY: 调用方契约（A9）；UnsafeCell::get 是共享引用→可变引用的
        // 唯一合法途径，返回值生命周期绑定本 `&self`。
        let any = unsafe { &mut *cell.get() };
        any.downcast_mut::<T>()
    }

    /// 是否已注册该类型资源。
    pub(crate) fn contains<T: Resource>(&self) -> bool {
        self.values.contains_key(&TypeId::of::<T>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct TestRes(u32);

    struct UnusedRes;

    #[test]
    fn insert_replaces_and_returns_old() {
        let mut map = ResourceMap::new();
        assert!(map.insert(TestRes(1)).is_none());
        // 同类型二次插入返回被替换的旧值
        assert_eq!(map.insert(TestRes(2)).map(|r| r.0), Some(1));
        assert_eq!(map.get::<TestRes>().map(|r| r.0), Some(2));
    }

    #[test]
    fn get_missing_returns_none() {
        let map = ResourceMap::new();
        assert!(map.get::<TestRes>().is_none());
    }

    #[test]
    fn get_mut_modifies_in_place() {
        let mut map = ResourceMap::new();
        map.insert(TestRes(1));
        if let Some(res) = map.get_mut::<TestRes>() {
            res.0 = 5;
        }
        assert_eq!(map.get::<TestRes>().map(|r| r.0), Some(5));
        // 未注册类型的 get_mut 返回 None
        assert!(map.get_mut::<UnusedRes>().is_none());
    }

    #[test]
    fn remove_missing_returns_none() {
        let mut map = ResourceMap::new();
        assert!(map.remove::<TestRes>().is_none());
        map.insert(TestRes(3));
        assert_eq!(map.remove::<TestRes>().map(|r| r.0), Some(3));
        // 移除后再次查询为 None
        assert!(map.remove::<TestRes>().is_none());
        assert!(map.get::<TestRes>().is_none());
    }

    #[test]
    fn contains_flags_registered() {
        let mut map = ResourceMap::new();
        assert!(!map.contains::<TestRes>());
        map.insert(TestRes(0));
        assert!(map.contains::<TestRes>());
        assert!(!map.contains::<UnusedRes>());
    }

    #[test]
    fn get_mut_unchecked_shared_ref_modifies_in_place() {
        let mut map = ResourceMap::new();
        map.insert(TestRes(7));
        // SAFETY: 测试中独占使用该资源，无并发访问（A9 契约在测试内成立）
        if let Some(res) = unsafe { map.get_mut_unchecked::<TestRes>() } {
            res.0 = 9;
        }
        assert_eq!(map.get::<TestRes>().map(|r| r.0), Some(9));
        // 未注册类型返回 None
        assert!(unsafe { map.get_mut_unchecked::<UnusedRes>() }.is_none());
    }
}
