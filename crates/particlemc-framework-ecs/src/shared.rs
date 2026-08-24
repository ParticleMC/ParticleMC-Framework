//! 共享只读资源：多世界共享同一 `Arc<T>`，零拷贝只读访问。
//!
//! 变更标识符：`implement-custom-ecs`
//!
//! IC-13 / R8.4：注册表、配置等全局只读数据以 [`Shared<T>`] 资源注入各
//! Instance 的 World——各世界持有同一 `Arc<T>`，读取无锁、零同步开销、
//! 零拷贝。本模块纯 safe（`Arc` 自身保证线程安全）。
//!
//! 系统参数侧（T7）以 `Res<Shared<T>>` 直接读取共享引用；本模块提供装配层
//! 入口 `World::insert_shared` / `World::shared`（IC-4 关联，T10 装配迁移
//! 使用）。

use std::ops::Deref;
use std::sync::Arc;

use crate::world::World;

/// 全局只读共享资源（IC-13）：`Arc<T>` 的透明包装。
///
/// 作为 `Resource` 存储于各 World（经 `World::insert_shared`），多世界共享
/// 同一底层分配；`Deref` 提供只读访问（**无 `DerefMut`**，编译期杜绝可变
/// 访问，满足 R8.4 "只读不可变"语义）。
pub struct Shared<T: ?Sized>(pub(crate) Arc<T>);

impl<T: ?Sized> Shared<T> {
    /// 由 `Arc` 构造共享资源。
    pub fn new(v: Arc<T>) -> Self {
        Shared(v)
    }

    /// 只读获取底层 `Arc` 引用。
    pub fn get(&self) -> &Arc<T> {
        &self.0
    }
}

impl<T: ?Sized> Clone for Shared<T> {
    /// 克隆共享引用（引用计数 +1，底层数据零拷贝）。
    fn clone(&self) -> Self {
        Shared(Arc::clone(&self.0))
    }
}

impl<T: Default> Default for Shared<T> {
    /// 由 `T::default()` 构造共享引用。
    ///
    /// 满足 `Res<Shared<T>>` 对 `Shared<T>: Resource + Default` 的约束
    /// （`system.rs` 的 `SystemParam for Res` 要求 `T: Default`）：`init_state`
    /// 经 `init_resource` 惰性补默认时插入空共享值，但运行期已由装配层注入
    /// 真实共享值（12.5.1 经 `World::insert_shared`），故默认值仅在缺省路径
    /// 生效、不覆盖真实值（与 `Res<T>` 的幂等语义一致）。
    fn default() -> Self {
        Shared(Arc::new(T::default()))
    }
}

impl<T: ?Sized> Deref for Shared<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl World {
    /// 注入全局只读共享资源（R8.4）：各世界持有同一 `Arc`，零拷贝只读。
    ///
    /// 等价于 `insert_resource(Shared(v))`（`Shared<T>` 作为 Resource 存储，
    /// 各类型全局唯一一份）。
    pub fn insert_shared<T: Send + Sync + 'static>(&mut self, v: Arc<T>) {
        self.insert_resource(Shared(v));
    }

    /// 只读获取全局共享资源引用；未注入返回 `None`。
    pub fn shared<T: Send + Sync + 'static>(&self) -> Option<&Arc<T>> {
        self.resource::<Shared<T>>().map(|s| &s.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试共享资源：模拟全局只读配置/注册表。
    #[derive(Debug, PartialEq, Eq)]
    struct Registry {
        entries: Vec<String>,
    }

    #[test]
    fn shared_insert_and_query() {
        let mut world = World::new();
        // 未注入：None
        assert!(world.shared::<Registry>().is_none());
        let arc = Arc::new(Registry {
            entries: vec!["spawn".to_string(), "config".to_string()],
        });
        world.insert_shared(Arc::clone(&arc));
        let got = world.shared::<Registry>().unwrap();
        // 同一 Arc（ptr_eq 而非值相等）
        assert!(Arc::ptr_eq(got, &arc));
        assert_eq!(got.entries, vec!["spawn".to_string(), "config".to_string()]);
    }

    #[test]
    fn shared_deref_reads_only() {
        // Deref 提供只读访问；无 DerefMut（编译期只读，R8.4）
        let mut world = World::new();
        world.insert_shared(Arc::new(Registry { entries: vec![] }));
        let shared = world.shared::<Registry>().unwrap();
        assert_eq!(shared.entries.len(), 0);
        // Shared<T> 独立类型 Deref 到 T
        let s = Shared::new(Arc::new(Registry {
            entries: vec!["x".to_string()],
        }));
        assert_eq!(s.entries, vec!["x".to_string()]);
    }

    #[test]
    fn shared_arc_shared_between_worlds() {
        // R8.4 语义：多世界共享同一 Arc，零拷贝（ptr_eq）
        let arc = Arc::new(Registry {
            entries: vec!["cfg".to_string()],
        });
        let mut w1 = World::new();
        let mut w2 = World::new();
        w1.insert_shared(Arc::clone(&arc));
        w2.insert_shared(Arc::clone(&arc));
        let r1 = w1.shared::<Registry>().unwrap();
        let r2 = w2.shared::<Registry>().unwrap();
        assert!(Arc::ptr_eq(r1, r2));
    }

    #[test]
    fn shared_clone_increments_refcount() {
        let arc = Arc::new(Registry { entries: vec![] });
        let s1 = Shared::new(Arc::clone(&arc));
        let s2 = s1.clone();
        assert!(Arc::ptr_eq(s1.get(), s2.get()));
        assert_eq!(Arc::strong_count(&arc), 3);
    }

    #[test]
    fn shared_get_returns_arc_reference() {
        let arc = Arc::new(Registry { entries: vec![] });
        let s = Shared::new(Arc::clone(&arc));
        assert!(Arc::ptr_eq(s.get(), &arc));
    }
}
