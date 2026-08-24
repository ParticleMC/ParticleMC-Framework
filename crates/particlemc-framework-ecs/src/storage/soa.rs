//! SoA（Structure of Arrays）组件列存储：热组件紧凑连续、缓存行对齐。
//!
//! 变更标识符：`implement-custom-ecs`
//!
//! 列与 Archetype 的 `slots` 严格同索引对齐（R3.1），`#[repr(align(64))]`
//! 包装避免多线程下相邻槽位的伪共享。本文件为 **unsafe 白名单**（U1）：
//! `get_unchecked`/`get_unchecked_mut` 与迭代器（[`Iter`]/[`IterMut`] 裸指针
//! 推进）为迭代热路径消除边界分支（R14.5），全部以 `# Safety` 文档 +
//! `debug_assert!` 护城河约束（索引必须先判界），其余 API 全部走安全路径。

#![allow(unsafe_code)]

use crate::storage::ErasedColumn;
use crate::util::Align64;

/// SoA 组件列：`Align64<Vec<T>>`，列容器按 64 字节对齐。
///
/// 列元数据包于 [`Align64`]（列本身不与其他列共享缓存行，避免伪共享，R3.1 /
/// T15.2）；底层 `Vec` 由全局分配器托管（元素缓冲对齐受 stable 工具链 +
/// `#![deny(unsafe_code)]` 约束，未启用自定义对齐分配器，详见 `docs/benchmarks.md`
/// 的"已知约束"）。内部数据包于 [`std::cell::UnsafeCell`]：Query 可变拆借（A8）
/// 经 `UnsafeCell::get`（共享→可变唯一合法途径）；安全方法读写在 `unsafe` 块内
/// 经 get() 重建引用。`Send + Sync` 由调用契约保证（单 World 单线程 tick）。
pub struct SoAColumn<T> {
    pub(crate) data: std::cell::UnsafeCell<Align64<Vec<T>>>,
}

impl<T> SoAColumn<T> {
    /// 空列。
    pub fn new() -> Self {
        SoAColumn {
            data: std::cell::UnsafeCell::new(Align64(Vec::new())),
        }
    }

    /// 预分配 `capacity` 容量的空列。
    pub fn with_capacity(capacity: usize) -> Self {
        SoAColumn {
            data: std::cell::UnsafeCell::new(Align64(Vec::with_capacity(capacity))),
        }
    }

    /// 以 `len` 个默认值填充创建列（首次 `insert<T>` 惰性建列时补齐与
    /// `slots` 对齐，中间实体获得默认值；需 `T: Default`）。
    pub fn with_defaults(len: usize) -> Self
    where
        T: Default,
    {
        let column = Self::with_capacity(len);
        for _ in 0..len {
            // 逐个 push 而非 Vec::resize：resize 要求 T: Clone，Default 不蕴含
            // Clone，push 仅需 T 本身
            unsafe { &mut *column.data.get() }.0.push(T::default());
        }
        column
    }

    /// 读取索引处元素的不可变引用；越界返回 `None`（安全 API，走边界检查）。
    pub fn get(&self, index: usize) -> Option<&T> {
        unsafe { &*self.data.get() }.0.get(index)
    }

    /// 读取索引处元素的可变引用；越界返回 `None`。
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        unsafe { &mut *self.data.get() }.0.get_mut(index)
    }

    /// 写入索引处元素；越界 panic（内部不变量：列始终与 `slots` 对齐）。
    pub fn set(&mut self, index: usize, value: T) {
        match unsafe { &mut *self.data.get() }.0.get_mut(index) {
            Some(slot) => *slot = value,
            // 不可达：World 保证列长度恒等于 slots 长度，index 来自 entity_index
            None => unreachable!(
                "SoA 列索引 {index} 越界（列长度 {}，违反与 slots 对齐不变量）",
                unsafe { &*self.data.get() }.0.len()
            ),
        }
    }

    /// 追加元素（列扩容为几何增长，见 `World::reserve`/Vec 自身策略）。
    pub fn push(&mut self, value: T) {
        unsafe { &mut *self.data.get() }.0.push(value);
    }

    /// 取走索引处元素并重置为该槽位的默认值；越界返回 `None`。
    ///
    /// 供 `World::remove<T>`（SoA 语义：组件"移除" = 重置默认，不破坏与
    /// `slots` 的紧凑对齐）。返回类型擦除值，由调用方 `downcast` 还原。
    pub fn take(&mut self, index: usize) -> Option<Box<dyn std::any::Any + Send + Sync>>
    where
        T: Default + Send + Sync + 'static,
    {
        let slot = unsafe { &mut *self.data.get() }.0.get_mut(index)?;
        Some(Box::new(std::mem::take(slot)))
    }

    /// 删除索引处元素（最后一个元素移动到被删位置保持紧凑）；越界 panic。
    pub fn swap_remove(&mut self, index: usize) -> T {
        unsafe { &mut *self.data.get() }.0.swap_remove(index)
    }

    /// 当前元素数（= 该 Archetype 的 `slots` 长度）。
    pub fn len(&self) -> usize {
        unsafe { &*self.data.get() }.0.len()
    }

    /// 是否为空列。
    pub fn is_empty(&self) -> bool {
        unsafe { &*self.data.get() }.0.is_empty()
    }

    /// 已分配容量。
    pub fn capacity(&self) -> usize {
        unsafe { &*self.data.get() }.0.capacity()
    }

    /// 额外预留 `additional` 个元素容量（预分配策略，R3.4）。
    pub fn reserve(&mut self, additional: usize) {
        unsafe { &mut *self.data.get() }.0.reserve(additional);
    }

    /// 只读迭代器：索引游标 + U1 无界访问，零堆分配（热路径，R14.5）。
    #[inline]
    pub fn iter(&self) -> Iter<'_, T> {
        Iter::new(self)
    }

    /// 可变迭代器：索引游标 + U1 无界访问，零堆分配。
    #[inline]
    pub fn iter_mut(&mut self) -> IterMut<'_, T> {
        IterMut::new(self)
    }

    /// 底层切片视图。
    pub fn as_slice(&self) -> &[T] {
        unsafe { &*self.data.get() }.0.as_slice()
    }

    /// 底层切片可变视图（A8：调用方保证单 World 单线程 tick 下独占访问）。
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        unsafe { &mut *self.data.get() }.0.as_mut_slice()
    }

    /// U1：无界读取——消除边界分支（R14.5 热路径无分支前提）。
    ///
    /// # Safety
    ///
    /// 调用方必须保证 `index < self.len()`；违规在 debug 构建下触发
    /// `debug_assert!`，release 构建下为未定义行为。返回引用的借用规则与
    /// 常规不可变借用一致：借用期内不得对列做任何可变操作。
    pub unsafe fn get_unchecked(&self, index: usize) -> &T {
        debug_assert!(index < self.len(), "get_unchecked 越界：{index}");
        // SAFETY: 调用方已保证 index < len，且借用规则满足
        unsafe {
            let slice = &(*self.data.get()).0;
            slice.get_unchecked(index)
        }
    }

    /// U1：无界写入——消除边界分支。
    ///
    /// # Safety
    ///
    /// 调用方必须保证 `index < self.len()`；违规在 debug 构建下触发
    /// `debug_assert!`，release 构建下为未定义行为。返回引用的借用规则与
    /// 常规可变借用一致：借用期内不得再以任何方式访问该列。
    pub unsafe fn get_unchecked_mut(&mut self, index: usize) -> &mut T {
        debug_assert!(index < self.len(), "get_unchecked_mut 越界：{index}");
        // SAFETY: 调用方已保证 index < len，且借用规则满足
        let ptr = unsafe { &mut *self.data.get() };
        unsafe { ptr.0.get_unchecked_mut(index) }
    }
}

/// 列只读迭代器：索引游标推进，热路径经 U1 无界访问（裸指针 `add` 等价于
/// `get_unchecked`），消除边界分支（R14.5）。零堆分配。
///
/// 生命周期标记 `PhantomData<&'a T>` 将迭代器的借用期绑定到列本身；裸指针
/// 仅在 `index < len` 时解引用（`next` 先判界再取址），越界访问不可达。
pub struct Iter<'a, T> {
    /// 指向列元素存储的起始指针（`Vec::as_ptr`，随列 `'a` 存活）。
    data: *const T,
    /// 元素总数（快照：迭代器持有列借用，迭代期间列长度不变）。
    len: usize,
    /// 游标：下一个待产出元素的下标（单调递增，保证每元素至多产出一次）。
    index: usize,
    marker: std::marker::PhantomData<&'a T>,
}

impl<'a, T> Iter<'a, T> {
    /// 由列构造只读迭代器。
    fn new(column: &'a SoAColumn<T>) -> Self {
        Iter {
            data: unsafe { &*column.data.get() }.0.as_ptr(),
            len: column.len(),
            index: 0,
            marker: std::marker::PhantomData,
        }
    }
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<&'a T> {
        if self.index >= self.len {
            return None;
        }
        let i = self.index;
        self.index += 1;
        // SAFETY: i < len（上面判界，索引在界内）；data 指向列存储且随列
        // 'a 存活；迭代器只读，与列的不可变借用不冲突
        unsafe { Some(&*self.data.add(i)) }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.len - self.index;
        (remaining, Some(remaining))
    }
}

impl<T> ExactSizeIterator for Iter<'_, T> {}

/// 列可变迭代器：语义与 [`Iter`] 一致，产出 `&'a mut T`。游标单调递增保证
/// 每元素至多产出一次，且迭代器持有列的独占 `&'a mut`，无重复可变借用。
pub struct IterMut<'a, T> {
    /// 指向列元素存储的起始指针（`Vec::as_mut_ptr`，随列 `'a` 存活）。
    data: *mut T,
    len: usize,
    index: usize,
    marker: std::marker::PhantomData<&'a mut T>,
}

impl<'a, T> IterMut<'a, T> {
    /// 由列构造可变迭代器。
    fn new(column: &'a mut SoAColumn<T>) -> Self {
        IterMut {
            data: unsafe { &mut *column.data.get() }.0.as_mut_ptr(),
            len: column.len(),
            index: 0,
            marker: std::marker::PhantomData,
        }
    }
}

impl<'a, T> Iterator for IterMut<'a, T> {
    type Item = &'a mut T;

    fn next(&mut self) -> Option<&'a mut T> {
        if self.index >= self.len {
            return None;
        }
        let i = self.index;
        self.index += 1;
        // SAFETY: i < len（上面判界）；data 指向列存储且随列 'a 存活；迭代器
        // 独占列借用，游标单调递增使每元素至多产出一次，无重复可变引用
        unsafe { Some(&mut *self.data.add(i)) }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.len - self.index;
        (remaining, Some(remaining))
    }
}

impl<T> ExactSizeIterator for IterMut<'_, T> {}

impl<T> Default for SoAColumn<T> {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: 内部 UnsafeCell 仅为 Query 可变拆借（A8）提供通道；所有访问由
// 调用方互斥契约保证（单 World 单线程 tick + query_mut 独占构造），跨线程
// 共享仅经只读访问（Sync 语义），不存在并发写。
unsafe impl<T: Send> Sync for SoAColumn<T> {}

/// `f32` 列的批量算术（位置 / 速度热路径），提供 SIMD（AVX2）与标量双路径
/// （U4 白名单 unsafe：T16.2）。非 x86_64 平台仅标量路径可用（SIMD 经
/// `cfg` 排除，调用方须提供回退，见 [`SoAColumn::add_assign_scalar`]）。
impl SoAColumn<f32> {
    /// 标量批量自增：`dst[i] += rhs[i]`（`rhs.len()` 超出部分忽略），SIMD 的
    /// 回退路径与对照基准。
    #[inline]
    #[allow(clippy::needless_range_loop)]
    pub fn add_assign_scalar(&mut self, rhs: &[f32]) {
        let n = rhs.len().min(self.len());
        let cell = self.data.get();
        for i in 0..n {
            // 列数据经 UnsafeCell 拆借可变引用（A8 独占访问契约）；借用而非移动
            unsafe { &mut *cell }.0[i] += rhs[i];
        }
    }

    /// SIMD（AVX2）批量自增：`dst[i] += rhs[i]`，8 路 `_mm256_add_ps` 处理，
    /// 尾部标量补齐（U4 白名单 unsafe：T16.2）。
    ///
    /// 仅 x86_64 + AVX2 可用（运行时特征经 `#[target_feature]` 动态分发，
    /// 不要求整个二进制编译于 AVX2）；非 x86_64 平台经 [`add_assign_scalar`]
    /// 回退。底层缓冲经 `loadu`/`storeu`（非对齐加载/存储）访问，兼容全局
    /// 分配器任意对齐（stable 工具链 + `#![deny(unsafe_code)]` 下未启用自定义
    /// 64 字节对齐分配器，详见 `docs/benchmarks.md` 的"已知约束"）。
    ///
    /// # Safety
    ///
    /// 调用方须保证 `rhs.len() <= self.len()`（避免越界写入）；本方法已对长度
    /// 取 `min` 防御，但越界读取 `rhs` 由调用方保证。
    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2")]
    pub unsafe fn add_assign_simd(&mut self, rhs: &[f32]) {
        use std::arch::x86_64::*;
        let n = rhs.len().min(self.len());
        if n == 0 {
            return;
        }
        let data = unsafe { &mut *self.data.get() }.0.as_mut_ptr();
        let rhs_ptr = rhs.as_ptr();
        let mut i = 0;
        // SAFETY: i + 8 <= n <= self.len()，data 指向列缓冲，rhs_ptr 长度经
        // min 约束；循环仅推进至 n，无越界；loadu/storeu 兼容任意对齐
        unsafe {
            while i + 8 <= n {
                let a = _mm256_loadu_ps(data.add(i));
                let b = _mm256_loadu_ps(rhs_ptr.add(i));
                let s = _mm256_add_ps(a, b);
                _mm256_storeu_ps(data.add(i), s);
                i += 8;
            }
            while i < n {
                *data.add(i) += *rhs_ptr.add(i);
                i += 1;
            }
        }
    }
}

/// `T: Default` 为 ErasedColumn 实现条件：SoA 列仅在 `World::insert<T>`
/// （要求 `T: Default`，AI Amendment A5）时创建，故实际存储的列必满足该约束。
impl<T: Default + Send + Sync + 'static> ErasedColumn for SoAColumn<T> {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    #[allow(clippy::mut_from_ref)]
    unsafe fn as_any_mut_unchecked(&self) -> &mut dyn std::any::Any {
        // SAFETY: 调用方契约保证独占访问（A8）。data 为唯一字段（地址与
        // self 相同），经 UnsafeCell::get（共享→可变唯一合法途径，避开
        // invalid_reference_casting lint）获得 *mut 后 cast 回容器指针。
        unsafe { &mut *(self.data.get().cast::<SoAColumn<T>>()) }
    }

    fn is_sparse(&self) -> bool {
        false
    }

    fn take_at(&mut self, archetype_index: usize) -> Option<Box<dyn std::any::Any + Send + Sync>> {
        // 与 slots 对齐的紧凑列：取走该槽并重置默认，元素总数不变
        self.take(archetype_index)
    }

    fn take_slot(&mut self, _entity_slot: u32) -> Option<Box<dyn std::any::Any + Send + Sync>> {
        None
    }

    fn insert_at(&mut self, index: usize, value: Box<dyn std::any::Any + Send + Sync>) -> bool {
        let value = match value.downcast::<T>() {
            Ok(value) => *value,
            // 类型不匹配：调用方组件 ID 与列类型不一致，视为框架 bug
            Err(_) => return false,
        };
        match unsafe { &mut *self.data.get() }.0.get_mut(index) {
            Some(slot) => {
                *slot = value;
                true
            }
            // 越界：列与 slots 未对齐（违反不变量），debug 构建下暴露
            None => {
                debug_assert!(
                    false,
                    "insert_at 索引 {index} 越界（列长 {}，违反与 slots 对齐不变量）",
                    unsafe { &*self.data.get() }.0.len()
                );
                false
            }
        }
    }

    fn push_default(&mut self) {
        unsafe { &mut *self.data.get() }.0.push(T::default());
    }

    fn on_despawn(&mut self, archetype_index: usize, _entity_slot: u32) {
        // 不可达索引（> len）由 debug_assert 暴露；release 下防御性忽略，
        // 避免越界 panic 击穿 despawn 的 bool 返回语义
        debug_assert!(archetype_index < unsafe { &*self.data.get() }.0.len());
        if archetype_index < unsafe { &*self.data.get() }.0.len() {
            unsafe { &mut *self.data.get() }
                .0
                .swap_remove(archetype_index);
        }
    }

    fn len(&self) -> usize {
        unsafe { &*self.data.get() }.0.len()
    }

    fn capacity(&self) -> usize {
        unsafe { &*self.data.get() }.0.capacity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_then_get_reads_values() {
        let mut column = SoAColumn::new();
        column.push(1u32);
        column.push(2u32);
        assert_eq!(column.len(), 2);
        assert_eq!(column.get(0), Some(&1));
        assert_eq!(column.get(1), Some(&2));
        assert_eq!(column.as_slice(), &[1, 2]);
    }

    #[test]
    fn get_out_of_bounds_returns_none() {
        let mut column = SoAColumn::<u32>::new();
        column.push(1);
        assert!(column.get(0).is_some());
        assert!(column.get(1).is_none());
        assert!(column.get(usize::MAX).is_none());
    }

    #[test]
    fn get_mut_modifies_in_place() {
        let mut column = SoAColumn::new();
        column.push(10u32);
        *column.get_mut(0).unwrap() += 5;
        assert_eq!(column.get(0), Some(&15));
        assert!(column.get_mut(1).is_none());
    }

    #[test]
    fn set_overwrites_value() {
        let mut column = SoAColumn::with_defaults(2);
        column.set(0, 7u32);
        assert_eq!(column.get(0), Some(&7));
        assert_eq!(column.get(1), Some(&0));
    }

    #[test]
    fn with_defaults_fills_default() {
        let column = SoAColumn::<u32>::with_defaults(3);
        assert_eq!(column.len(), 3);
        assert_eq!(column.as_slice(), &[0u32, 0, 0]);
    }

    #[test]
    fn swap_remove_keeps_compaction() {
        let mut column = SoAColumn::new();
        column.push(1u32);
        column.push(2u32);
        column.push(3u32);
        let removed = column.swap_remove(0);
        assert_eq!(removed, 1);
        // 末尾元素移动到被删位置，保持紧凑
        assert_eq!(column.as_slice(), &[3, 2]);
        assert_eq!(column.len(), 2);
    }

    #[test]
    fn iter_and_iter_mut_yield_all_values() {
        let mut column = SoAColumn::new();
        column.push(1u32);
        column.push(2u32);
        column.push(3u32);
        let collected: Vec<u32> = column.iter().copied().collect();
        assert_eq!(collected, vec![1, 2, 3]);
        for value in column.iter_mut() {
            *value *= 10;
        }
        assert_eq!(column.as_slice(), &[10, 20, 30]);
    }

    #[test]
    fn iter_exact_size_and_order() {
        let mut column = SoAColumn::new();
        column.push(5u32);
        column.push(6u32);
        let mut iter = column.iter();
        assert_eq!(iter.len(), 2); // ExactSizeIterator：剩余元素数
        assert_eq!(iter.next(), Some(&5));
        assert_eq!(iter.len(), 1);
        assert_eq!(iter.next(), Some(&6));
        assert_eq!(iter.next(), None);
        assert_eq!(iter.len(), 0);
        // 空列迭代：立即结束，无解引用
        let mut empty = SoAColumn::<u32>::new();
        assert_eq!(empty.iter().count(), 0);
        assert_eq!(empty.iter_mut().count(), 0);
    }

    #[test]
    fn reserve_preallocates_capacity() {
        let mut column = SoAColumn::<u32>::new();
        assert_eq!(column.capacity(), 0);
        column.reserve(64);
        assert!(column.capacity() >= 64);
        assert_eq!(column.len(), 0);
        // 预留容量内 push 不触发扩容
        let cap = column.capacity();
        for i in 0..64 {
            column.push(i);
        }
        assert_eq!(column.capacity(), cap);
    }

    #[test]
    fn erase_insert_at_writes_and_rejects_mismatch() {
        let mut column = SoAColumn::<u32>::new();
        column.push(1);
        column.push(2);
        // 正常写入既有索引
        let boxed: Box<dyn std::any::Any + Send + Sync> = Box::new(42u32);
        assert!(column.insert_at(1, boxed));
        assert_eq!(column.get(1), Some(&42));
        // 类型不匹配（downcast 失败）：返回 false 且不产生部分写入
        let wrong: Box<dyn std::any::Any + Send + Sync> = Box::new("x".to_string());
        assert!(!column.insert_at(0, wrong));
        assert_eq!(column.get(0), Some(&1));
        // 越界索引：见 erase_insert_at_out_of_bounds_panics_in_debug
        // （debug 构建下越界触发断言而非返回 false，release 下返回 false）
        if !cfg!(debug_assertions) {
            let boxed: Box<dyn std::any::Any + Send + Sync> = Box::new(7u32);
            assert!(!column.insert_at(5, boxed));
        }
    }

    #[test]
    fn erase_insert_at_out_of_bounds_panics_in_debug() {
        // debug 构建下越界写入应触发断言（违反列与 slots 对齐不变量）
        let mut column = SoAColumn::<u32>::new();
        column.push(1);
        if cfg!(debug_assertions) {
            let boxed: Box<dyn std::any::Any + Send + Sync> = Box::new(7u32);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                column.insert_at(3, boxed);
            }));
            assert!(result.is_err());
        }
    }

    #[test]
    fn take_resets_to_default_and_returns_old() {
        let mut column = SoAColumn::<u32>::new();
        column.push(42);
        let taken = column
            .take(0)
            .and_then(|v| v.downcast::<u32>().ok().map(|b| *b));
        assert_eq!(taken, Some(42));
        // 槽位重置为默认值，元素总数不变（与 slots 对齐）
        assert_eq!(column.get(0), Some(&0));
        assert_eq!(column.len(), 1);
        // 越界返回 None
        assert!(column.take(5).is_none());
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic]
    fn get_unchecked_out_of_bounds_panics() {
        let mut column = SoAColumn::new();
        column.push(1u32);
        // debug 构建下越界访问必须触发 debug_assert
        let _ = unsafe { column.get_unchecked(5) };
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic]
    fn get_unchecked_mut_out_of_bounds_panics() {
        let mut column = SoAColumn::<u32>::new();
        let _ = unsafe { column.get_unchecked_mut(3) };
    }

    #[test]
    fn get_unchecked_in_bounds_reads_and_writes() {
        let mut column = SoAColumn::new();
        column.push(1u32);
        column.push(2u32);
        // SAFETY: 索引均在 len() 内（测试用例），满足前置条件
        let value = unsafe { column.get_unchecked(0) };
        assert_eq!(*value, 1);
        let slot = unsafe { column.get_unchecked_mut(1) };
        *slot = 9;
        assert_eq!(column.get(1), Some(&9));
    }

    #[test]
    fn column_aligns_to_cache_line() {
        // R3.1：列容器（含底层 Vec 元数据）按缓存行（64 字节）对齐，避免相邻
        // 列元数据伪共享
        let column = SoAColumn::<u32>::new();
        assert_eq!(std::mem::align_of_val(unsafe { &*column.data.get() }), 64);
    }

    #[test]
    fn column_container_aligned_and_no_false_sharing() {
        // T15.2（可达部分）：列容器按 64 字节对齐，且数组中相邻列落在不同缓存
        // 行（地址步距 ≥ 64），避免多线程/多列迭代时的伪共享（R3.1）。
        // 注：元素缓冲（堆）的 64 字节对齐受 stable 工具链 + `#![deny(unsafe_code)]`
        // 约束未启用自定义对齐分配器，作为已知受限项记录于 `docs/benchmarks.md`。
        let a = SoAColumn::<u32>::new();
        assert_eq!(std::mem::align_of_val(&a), 64, "SoA 列容器未按 64 字节对齐");
        // 数组中两列：地址差应 ≥ 64（各自独立缓存行，无伪共享）
        let arr = [SoAColumn::<u32>::new(), SoAColumn::<u32>::new()];
        let pa = &arr[0] as *const _ as usize;
        let pb = &arr[1] as *const _ as usize;
        assert!(
            (pb - pa) >= 64,
            "相邻 SoA 列未隔离到独立缓存行（步距 {} < 64，伪共享风险）",
            pb - pa
        );
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    #[allow(clippy::needless_range_loop)]
    fn simd_add_matches_scalar() {
        // T16.2：SIMD 批量自增与标量路径结果一致（正确性优先于性能）
        let n = 100; // 非 8 倍数，验证尾部标量补齐
        let mut simd_col = SoAColumn::<f32>::with_defaults(n);
        let mut scalar_col = SoAColumn::<f32>::with_defaults(n);
        let mut base = Vec::with_capacity(n);
        let mut rhs = Vec::with_capacity(n);
        for i in 0..n {
            base.push(i as f32);
            rhs.push((i % 7) as f32);
        }
        for i in 0..n {
            simd_col.set(i, base[i]);
            scalar_col.set(i, base[i]);
        }
        // SAFETY: rhs.len() == self.len() == n，满足 add_assign_simd 前置
        unsafe { simd_col.add_assign_simd(&rhs) };
        scalar_col.add_assign_scalar(&rhs);
        for i in 0..n {
            assert_eq!(
                simd_col.get(i).unwrap(),
                scalar_col.get(i).unwrap(),
                "SIMD 与标量在第 {i} 个元素结果不一致"
            );
        }
        // 逐元素加法基准：SIMD/标量结果应等于 base[i] + rhs[i]
        if std::arch::is_x86_feature_detected!("avx2") {
            for i in 0..n {
                assert_eq!(*simd_col.get(i).unwrap(), base[i] + rhs[i]);
            }
        }
    }
}
