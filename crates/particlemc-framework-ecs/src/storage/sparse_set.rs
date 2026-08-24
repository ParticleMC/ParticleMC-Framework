//! SparseSet 独立存储：冷/高频变动组件的 O(1) 增删。
//!
//! 变更标识符：`implement-custom-ecs`
//!
//! 按**实体槽位**（`Entity::slot().0`）索引（R3.2）：插入/删除/查询均 O(1)，
//! 增删不触发 Archetype 搬迁（R3.3）。槽位越界时自动扩容（`Vec` 几何增长）。
//! 列内容随 `values` 的前缀 Option 稀疏分布，`len` 维护实际元素数。

// unsafe 白名单（A10 扩展）：内部 UnsafeCell 为 Query 可变拆借（A8）提供
// 通道，安全方法在 unsafe 块内经 `UnsafeCell::get` 重建引用。
#![allow(unsafe_code)]

use crate::storage::ErasedColumn;

/// SparseSet 组件列：`values` 为按槽位索引的 Option 数组，`count` 为实际
/// 元素数（跳过 `None` 槽位）。
///
/// 内部数据包于 [`std::cell::UnsafeCell`]：Query 可变拆借（A8）经
/// `UnsafeCell::get`；安全方法读写在 `unsafe` 块内经 get() 重建引用。
pub struct SparseSet<T> {
    values: std::cell::UnsafeCell<Vec<Option<T>>>,
    count: usize,
}

impl<T> SparseSet<T> {
    /// 空集合。
    pub fn new() -> Self {
        SparseSet {
            values: std::cell::UnsafeCell::new(Vec::new()),
            count: 0,
        }
    }

    /// 预分配 `capacity` 个槽位容量的空集合。
    pub fn with_capacity(capacity: usize) -> Self {
        SparseSet {
            values: std::cell::UnsafeCell::new(Vec::with_capacity(capacity)),
            count: 0,
        }
    }

    /// 在槽位写入值；槽位越界自动扩容（中间槽位补 `None`）。已占用槽位为
    /// 覆盖更新（幂等：同槽位二次写入不改变元素数）。
    pub fn insert(&mut self, slot: usize, value: T) {
        if slot >= unsafe { &*self.values.get() }.len() {
            // 逐个 push 而非 Vec::resize：resize 要求 T: Clone（Option<T>:
            // Clone），SparseSet 语义不应强加该约束
            while unsafe { &*self.values.get() }.len() <= slot {
                unsafe { &mut *self.values.get() }.push(None);
            }
        }
        let entry = match unsafe { &mut *self.values.get() }.get_mut(slot) {
            Some(entry) => entry,
            // 不可达：上面扩容后 slot 必在界内
            None => unreachable!("扩容后槽位 {slot} 必然在界"),
        };
        if entry.is_none() {
            self.count += 1;
        }
        *entry = Some(value);
    }

    /// 删除槽位值并返回；槽位越界或为空返回 `None`（幂等：二次删除返回
    /// `None`，不改变状态）。
    pub fn remove(&mut self, slot: usize) -> Option<T> {
        match unsafe { &mut *self.values.get() }.get_mut(slot) {
            Some(entry) => {
                let old = entry.take();
                if old.is_some() {
                    self.count -= 1;
                }
                old
            }
            None => None,
        }
    }

    /// 只读获取槽位值；槽位越界或为空返回 `None`。
    pub fn get(&self, slot: usize) -> Option<&T> {
        unsafe { &*self.values.get() }
            .get(slot)
            .and_then(Option::as_ref)
    }

    /// 可变获取槽位值；槽位越界或为空返回 `None`。
    pub fn get_mut(&mut self, slot: usize) -> Option<&mut T> {
        unsafe { &mut *self.values.get() }
            .get_mut(slot)
            .and_then(Option::as_mut)
    }

    /// 实际元素数（占用槽位计数）。
    pub fn len(&self) -> usize {
        self.count
    }

    /// 是否无元素。
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// 已分配槽位容量（内存统计，R13.2 数据源）。
    pub fn capacity(&self) -> usize {
        unsafe { &*self.values.get() }.capacity()
    }

    /// 额外预留 `additional` 个槽位容量（预分配策略，R3.4）。
    pub fn reserve(&mut self, additional: usize) {
        unsafe { &mut *self.values.get() }.reserve(additional);
    }

    /// 遍历全部值（跳过 `None` 槽位），顺序为槽位升序。
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        unsafe { &*self.values.get() }.iter().flatten()
    }
}

impl<T> Default for SparseSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: 内部 UnsafeCell 仅为 Query 可变拆借（A8）提供通道；所有访问由
// 调用方互斥契约保证（单 World 单线程 tick + query_mut 独占构造），跨线程
// 共享仅经只读访问（Sync 语义），不存在并发写。
unsafe impl<T: Send> Sync for SparseSet<T> {}

impl<T: Send + Sync + 'static> ErasedColumn for SparseSet<T> {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    unsafe fn as_any_mut_unchecked(&self) -> &mut dyn std::any::Any {
        // SAFETY: 调用方契约保证独占访问（A8）。values 为首字段（地址与
        // self 相同），经 UnsafeCell::get（共享→可变唯一合法途径，避开
        // invalid_reference_casting lint）获得 *mut 后 cast 回容器指针。
        unsafe { &mut *(self.values.get().cast::<SparseSet<T>>()) }
    }

    fn is_sparse(&self) -> bool {
        true
    }

    fn take_at(&mut self, _archetype_index: usize) -> Option<Box<dyn std::any::Any + Send + Sync>> {
        // Sparse 列按实体槽位索引，不由 Archetype 槽位对齐
        None
    }

    fn take_slot(&mut self, entity_slot: u32) -> Option<Box<dyn std::any::Any + Send + Sync>> {
        // u32 → usize：64 位平台为扩宽转换（无损），`as usize` 非缩窄；std
        // 未实现 `From<u32> for usize`（理论 16 位平台不成立），故用 as
        self.remove(entity_slot as usize)
            .map(|value| Box::new(value) as Box<dyn std::any::Any + Send + Sync>)
    }

    fn insert_at(&mut self, index: usize, value: Box<dyn std::any::Any + Send + Sync>) -> bool {
        match value.downcast::<T>() {
            Ok(value) => {
                // index 为实体槽位：越界自动扩容，符合 SparseSet 语义
                self.insert(index, *value);
                true
            }
            // 类型不匹配：downcast 失败，不写入
            Err(_) => false,
        }
    }

    fn push_default(&mut self) {
        // Sparse 按槽位索引存储，不随 Archetype 槽位增长（World::spawn 时
        // 不预占槽位），故 no-op
    }

    fn on_despawn(&mut self, _archetype_index: usize, entity_slot: u32) {
        // 实体销毁时清掉其槽位值，防止悬挂残留
        self.remove(entity_slot as usize);
    }

    fn len(&self) -> usize {
        self.count
    }

    fn capacity(&self) -> usize {
        unsafe { &*self.values.get() }.capacity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_then_get_roundtrip() {
        let mut set = SparseSet::new();
        set.insert(3, "a");
        set.insert(7, "b");
        assert_eq!(set.len(), 2);
        assert_eq!(set.get(3), Some(&"a"));
        assert_eq!(set.get(7), Some(&"b"));
        assert_eq!(set.get(0), None);
    }

    #[test]
    fn get_mut_modifies_in_place() {
        let mut set = SparseSet::new();
        set.insert(2, 10u32);
        *set.get_mut(2).unwrap() += 5;
        assert_eq!(set.get(2), Some(&15));
        // 越界 get_mut 返回 None
        assert!(set.get_mut(99).is_none());
    }

    #[test]
    fn remove_returns_old_and_is_idempotent() {
        let mut set = SparseSet::new();
        set.insert(1, 7u32);
        assert_eq!(set.remove(1), Some(7));
        assert_eq!(set.len(), 0);
        // 二次删除返回 None，不改变状态
        assert_eq!(set.remove(1), None);
        // 越界删除返回 None
        assert_eq!(set.remove(100), None);
        assert!(set.is_empty());
    }

    #[test]
    fn insert_overwrites_existing_without_double_count() {
        let mut set = SparseSet::new();
        set.insert(0, 1u32);
        set.insert(0, 2u32);
        // 同槽位覆盖更新：元素数不变（幂等）
        assert_eq!(set.len(), 1);
        assert_eq!(set.get(0), Some(&2));
    }

    #[test]
    fn auto_grows_when_slot_out_of_bounds() {
        let mut set = SparseSet::new();
        assert_eq!(set.capacity(), 0);
        set.insert(10, "x");
        // 越界槽位自动扩容，中间槽位保持空
        assert_eq!(unsafe { &*set.values.get() }.len(), 11);
        assert_eq!(set.get(10), Some(&"x"));
        assert_eq!(set.get(9), None);
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn iter_yields_values_in_slot_order() {
        let mut set = SparseSet::new();
        set.insert(5, 5u32);
        set.insert(1, 1u32);
        set.insert(5, 6u32); // 覆盖 5
        let got: Vec<u32> = set.iter().copied().collect();
        assert_eq!(got, vec![1, 6]);
    }

    #[test]
    fn with_capacity_and_reserve_preallocate() {
        let mut set = SparseSet::<u32>::with_capacity(32);
        assert!(set.capacity() >= 32);
        let cap = set.capacity();
        for i in 0..32 {
            set.insert(i, i as u32);
        }
        // 预分配容量内插入不扩容
        assert_eq!(set.capacity(), cap);
        set.reserve(16);
        assert!(set.capacity() >= 48);
    }

    #[test]
    fn erase_despawn_cleans_slot_value() {
        let mut set = SparseSet::new();
        set.insert(0, 1u32);
        set.insert(1, 2u32);
        set.on_despawn(0, 0); // 销毁槽位 0 的实体
        assert_eq!(set.get(0), None);
        assert_eq!(set.get(1), Some(&2));
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn erase_take_slot_returns_boxed_value() {
        let mut set = SparseSet::new();
        set.insert(2, 9u32);
        let boxed = set.take_slot(2).unwrap();
        let value = boxed.downcast::<u32>().ok().map(|b| *b);
        assert_eq!(value, Some(9));
        assert!(set.get(2).is_none());
        assert!(set.take_slot(2).is_none());
    }

    #[test]
    fn erase_insert_at_roundtrip() {
        // 显式注解 T=u32：insert_at 的 downcast 目标类型由 SparseSet 的 T 决定
        let mut set = SparseSet::<u32>::new();
        // 正常写入（槽位为索引）
        let boxed: Box<dyn std::any::Any + Send + Sync> = Box::new(9u32);
        assert!(set.insert_at(3, boxed));
        assert_eq!(set.get(3), Some(&9));
        // 类型不匹配：downcast 失败，不写入
        let wrong: Box<dyn std::any::Any + Send + Sync> = Box::new("s".to_string());
        assert!(!set.insert_at(3, wrong));
        assert_eq!(set.get(3), Some(&9));
        // 越界槽位自动扩容
        let boxed: Box<dyn std::any::Any + Send + Sync> = Box::new(5u32);
        assert!(set.insert_at(100, boxed));
        assert_eq!(set.get(100), Some(&5));
        assert_eq!(set.len(), 2);
    }
}
