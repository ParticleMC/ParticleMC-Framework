//! 独立 ECS 世界。
//!
//! 变更标识符：`implement-custom-ecs`
//!
//! T1 提供资源存储（`resources` 字段 + Resource 方法族）；T3 补齐实体存储：
//! `entities`（槽位分配器）、`archetypes`（按 ArchetypeId 索引的存储）、
//! `entity_index`（实体 → Archetype + 槽位索引的反查表），并实现 IC-4 的
//! spawn/despawn/contains/insert/remove/get/get_mut/entity_count/
//! entities_by_kind/component_capacity。
//!
//! 组件列**惰性创建**：首次 `insert<T>` 时创建对应列（此时具备具体类型 T），
//! 之后 `get::<T>` 对从未 insert 过的 SoA 组件返回 `None`（"组件不存在"
//! 语义）。列创建时以 `T::default()` 补齐到与 `slots` 对齐，后续 `spawn` 对
//! 既有 SoA 列逐列 push 默认占位，故列长度恒等于 `slots` 长度。
//!
//! T8 追加跨线程/迁移辅助方法：`entity_archetype`（迁移查询）、`spawn_exact`
//! （保留 ID 落位）、`insert_any`（类型擦除组件写入），供
//! [`crate::migration::migrate_entity`] 使用（IC-12）；`insert_shared`/
//! `shared` 定义于 [`crate::shared`]（IC-13）。

use std::any::Any;
use std::collections::HashMap;

use crate::archetype::{ArchetypeDef, ArchetypeId};
use crate::component::{Component, ComponentId, ComponentStorage};
use crate::entity::{Entity, EntityArena, EntityTypeId};
use crate::message::{Message, MessageInbox};
use crate::resource::{Resource, ResourceMap};
use crate::storage::ArchetypeStorage;
use crate::storage::soa::SoAColumn;
use crate::storage::sparse_set::SparseSet;

/// 实体操作错误（IC-4，AI Amendment A4 增加 ComponentMismatch 变体）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityError {
    /// 实体不存在：从未 spawn、已 despawn，或世代不匹配的悬挂句柄。
    ///
    /// despawn 后的实体操作统一归入本变体（`is_alive` 失败即返回）。
    NotFound,
    /// 实体已销毁（文档性变体）：运行时不可达，销毁后的操作由
    /// [`EntityError::NotFound`] 覆盖。保留以对齐 IC-4 冻结契约。
    Despawned,
    /// SoA 组件不属于该实体所属 Archetype 的固定组件集合（R3.3：SoA 组件
    /// 增删受 archetype 组件集约束；Sparse 组件任意增删，不受本错误约束）。
    ComponentMismatch,
}

/// 组件束：可一次性生成进世界（旧 ECS 方案 `Bundle` 迁移等价，T11）。
///
/// 单组件 `T: Component` 与元组 `(A, B, ...)`（各 `Component`）均实现本 trait；
/// [`World::spawn_bundle`] 以之生成空实体并写入全部组件，返回 [`SpawnBuilder`]
/// 以支持链式 `insert` 与 `.id()` 取实体句柄，对齐 旧 ECS 方案 的 `spawn`/`spawn_empty`
/// 调用形态，最大限度减少迁移期调用点改动。
pub trait Bundle {
    /// 将自身组件写入世界，返回新实体句柄。
    fn spawn(self, world: &mut World) -> Entity;
}

impl<T: Component + Default + Send + Sync> Bundle for T {
    fn spawn(self, world: &mut World) -> Entity {
        let e = world.spawn_empty_with(EntityTypeId(0));
        let _ = world.insert(e, self);
        e
    }
}

macro_rules! impl_bundle_for_tuple {
    ($($name:ident),+ $(,)?) => {
        #[allow(non_snake_case)]
        impl<$($name: Component + Default + Send + Sync),+> Bundle for ($($name,)+) {
            fn spawn(self, world: &mut World) -> Entity {
                let e = world.spawn_empty_with(EntityTypeId(0));
                let ($($name,)+) = self;
                $( let _ = world.insert(e, $name); )+
                e
            }
        }
    };
}

impl_bundle_for_tuple!(A);
impl_bundle_for_tuple!(A, B);
impl_bundle_for_tuple!(A, B, C);
impl_bundle_for_tuple!(A, B, C, D);
impl_bundle_for_tuple!(A, B, C, D, E);
impl_bundle_for_tuple!(A, B, C, D, E, F);
impl_bundle_for_tuple!(A, B, C, D, E, F, G);
impl_bundle_for_tuple!(A, B, C, D, E, F, G, H);
impl_bundle_for_tuple!(A, B, C, D, E, F, G, H, I);
impl_bundle_for_tuple!(A, B, C, D, E, F, G, H, I, J);
impl_bundle_for_tuple!(A, B, C, D, E, F, G, H, I, J, K);
impl_bundle_for_tuple!(A, B, C, D, E, F, G, H, I, J, K, L);

/// 生成构建器：链式 `insert` 追加组件，`.id()` 取实体（旧 ECS 方案 `EntityCommands`
/// 迁移等价，T11）。持有 `&mut World` 借用直至 `.id()` 或末次 `.insert()` 消费。
pub struct SpawnBuilder<'w> {
    world: &'w mut World,
    entity: Entity,
}

impl<'w> SpawnBuilder<'w> {
    /// 追加一个组件（Sparse 任意增删；SoA 受 archetype 固定集约束）。
    pub fn insert<T: Component + Default + Send + Sync>(self, component: T) -> Self {
        let _ = self.world.insert(self.entity, component);
        self
    }

    /// 取实体句柄（消费构建器）。
    pub fn id(self) -> Entity {
        self.entity
    }
}

/// 独立 ECS 世界：资源表 + 实体存储 + Archetype 存储。
///
/// 字段 `pub(crate)`：供兄弟模块（T4 query / T5 commands / T8 migration）
/// 扩展 impl（R11 每个 Instance 持有独立 World）。
pub struct World {
    pub(crate) resources: ResourceMap,
    /// 按实体类型隔离的槽位分配器。
    pub(crate) entities: EntityArena,
    /// 按 ArchetypeId 索引的存储（注册于 `register_archetype`）。
    pub(crate) archetypes: HashMap<ArchetypeId, ArchetypeStorage>,
    /// 实体 → (ArchetypeId, 槽位索引)：O(1) 反查，despawn 时同步维护。
    pub(crate) entity_index: HashMap<Entity, (ArchetypeId, usize)>,
    /// 合成空 Archetype 映射（每实体类型一个，供 `spawn_empty` 惰性注册，T11 迁移）。
    pub(crate) empty_archetypes: HashMap<EntityTypeId, ArchetypeId>,
    /// 合成 ArchetypeId 计数器（高位起算，避免与静态注册 ID 冲突）。
    pub(crate) synthetic_id_counter: u16,
}

impl World {
    /// 空世界（无任何资源与实体）。
    pub fn new() -> Self {
        World {
            resources: ResourceMap::new(),
            entities: EntityArena::new(),
            archetypes: HashMap::new(),
            entity_index: HashMap::new(),
            empty_archetypes: HashMap::new(),
            synthetic_id_counter: 0x8000,
        }
    }

    /// 注册静态 Archetype 定义（IC-3/R2.3）。
    ///
    /// # Panics
    ///
    /// 重复注册同一 `ArchetypeId` 在 debug 构建下断言失败（静态 Archetype
    /// 的组件集合必须唯一，R2.1）；release 构建下以新定义覆盖。
    pub fn register_archetype(&mut self, def: &'static ArchetypeDef) {
        debug_assert!(
            !self.archetypes.contains_key(&def.id),
            "Archetype `{}`（ArchetypeId({})）重复注册",
            def.name,
            def.id.0
        );
        let storage = ArchetypeStorage {
            def: *def,
            slots: Vec::new(),
            columns: HashMap::new(),
        };
        self.archetypes.insert(def.id, storage);
    }

    /// 在指定 Archetype 中创建一个实体（组件初始为各列的默认值占位）。
    ///
    /// # Panics
    ///
    /// `arch` 未注册（从未 `register_archetype`）时 panic（IC-4：`spawn`
    /// 返回 `Entity` 而非 Result，错误状态以 panic 表达，文档注明）。
    pub fn spawn(&mut self, arch: ArchetypeId) -> Entity {
        let entity_kind = match self.archetypes.get(&arch) {
            Some(storage) => storage.def.entity_kind,
            None => panic!("未注册 Archetype：ArchetypeId({})", arch.0),
        };
        let entity = self.entities.allocate(entity_kind);
        let storage = match self.archetypes.get_mut(&arch) {
            Some(storage) => storage,
            // 不可达：上面已验证注册
            None => panic!("未注册 Archetype：ArchetypeId({})", arch.0),
        };
        let idx = storage.slots.len();
        // 既有 SoA 列逐列 push 默认占位，保持列长度 == slots 长度（Sparse 列
        // 按槽位索引，push_default 为 no-op）
        for column in storage.columns.values_mut() {
            column.push_default();
        }
        storage.slots.push(entity);
        self.entity_index.insert(entity, (arch, idx));
        entity
    }

    /// 创建一个不含任何组件的实体（T11 迁移：旧 ECS 方案 `spawn_empty` 等价）。
    ///
    /// 按实体类型惰性注册一个合成空 Archetype（无 SoA 组件；迁移期全部组件经
    /// Sparse 存储任意增删，不受静态 Archetype 组件集约束），随后 `spawn` 落入
    /// 该 Archetype。返回即可用的实体句柄；组件经 [`World::insert`] 追加。
    pub(crate) fn spawn_empty_with(&mut self, kind: EntityTypeId) -> Entity {
        let arch = *self.empty_archetypes.entry(kind).or_insert_with(|| {
            let id = ArchetypeId(self.synthetic_id_counter);
            self.synthetic_id_counter = self.synthetic_id_counter.wrapping_add(1);
            let def = ArchetypeDef {
                id,
                name: "EmptyArchetype",
                component_ids: &[],
                entity_kind: kind,
                component_types: &[],
            };
            self.archetypes.insert(
                id,
                ArchetypeStorage {
                    def,
                    slots: Vec::new(),
                    columns: HashMap::new(),
                },
            );
            id
        });
        self.spawn(arch)
    }

    /// 生成不含任何组件的实体并返回构建器（T11 迁移：旧 ECS 方案 `spawn_empty`
    /// 等价，默认实体类型 `EntityTypeId(0)`；particlemc-framework-core 不区分实体类型）。
    ///
    /// 链式 `.insert(...).id()` 追加组件并取句柄，对齐 旧 ECS 方案 `spawn_empty().id()`。
    pub fn spawn_empty(&mut self) -> SpawnBuilder<'_> {
        let entity = self.spawn_empty_with(EntityTypeId(0));
        SpawnBuilder {
            world: self,
            entity,
        }
    }

    /// 以组件束生成实体并返回构建器（T11 迁移：旧 ECS 方案 `spawn(组件/元组)` 等价）。
    ///
    /// 单组件与元组均由 [`Bundle`] 实现覆盖；返回 [`SpawnBuilder`] 以支持链式
    /// `.insert(...)` 与 `.id()`，最大限度复用 旧 ECS 方案 调用形态而无需逐点改写。
    pub fn spawn_bundle<B: Bundle>(&mut self, bundle: B) -> SpawnBuilder<'_> {
        let entity = bundle.spawn(self);
        SpawnBuilder {
            world: self,
            entity,
        }
    }

    /// 枚举全部存活实体（T11 迁移：旧 ECS 方案 `world.archetypes().iter().flat_map(...)`
    /// 的等价，替代实体表遍历；供需要全量枚举的系统如 `entity_ai` 使用）。
    pub fn entities(&self) -> Vec<Entity> {
        self.entity_index.keys().copied().collect()
    }

    /// 销毁实体并清理其全部组件；句柄无效（未 spawn/已销毁/悬挂）返回
    /// `false`。
    ///
    /// SoA 列经 `swap_remove` 保持紧凑（末尾实体移到被删槽位），`entity_index`
    /// 同步更新被交换实体的索引；Sparse 列移除该实体槽位值。
    pub fn despawn(&mut self, e: Entity) -> bool {
        let (arch, idx) = match self.entity_index.get(&e) {
            Some(&entry) => entry,
            None => return false,
        };
        let storage = match self.archetypes.get_mut(&arch) {
            Some(storage) => storage,
            // 不可达：entity_index 与 archetypes 由不变量保证一致
            None => return false,
        };
        let entity_slot = e.slot().0;
        // swap_remove 返回被移除的元素；移动到 idx 处的是旧末尾实体，需在
        // 交换后从 slots[idx] 读取
        let _ = storage.slots.swap_remove(idx);
        for column in storage.columns.values_mut() {
            column.on_despawn(idx, entity_slot);
        }
        self.entity_index.remove(&e);
        if let Some(&moved) = storage.slots.get(idx) {
            // 被删元素非末尾时，swap_remove 将末尾实体移到 idx 处，同步其
            // 新槽位索引（entity_index 与 slots 必须一致）
            self.entity_index.insert(moved, (arch, idx));
        }
        self.entities.deallocate(e);
        true
    }

    /// 句柄是否存活（已 spawn 且未 despawn，世代匹配）。
    pub fn contains(&self, e: Entity) -> bool {
        self.entities.is_alive(e)
    }

    /// 写入组件值。
    ///
    /// - **Sparse 组件**：任意实体均可增删（R3.3），列不存在则创建；不受
    ///   archetype 组件集约束。
    /// - **SoA 组件**：必须属于实体所属 Archetype（`ComponentMismatch`）；
    ///   列不存在则惰性创建并以 `T::default()` 补齐对齐（AI Amendment A5：
    ///   `T: Default` 约束；未显式 insert 的中间实体获得默认值）。
    ///
    /// # Errors
    ///
    /// - 实体不存在/已销毁 → [`EntityError::NotFound`]。
    /// - SoA 组件不属于该 Archetype → [`EntityError::ComponentMismatch`]。
    pub fn insert<T: Component + Default + Send + Sync>(
        &mut self,
        e: Entity,
        c: T,
    ) -> Result<(), EntityError> {
        if !self.entities.is_alive(e) {
            return Err(EntityError::NotFound);
        }
        let (arch, idx) = match self.entity_index.get(&e) {
            Some(&entry) => entry,
            None => return Err(EntityError::NotFound),
        };
        let storage = match self.archetypes.get_mut(&arch) {
            Some(storage) => storage,
            // 不可达：entity_index 引用的 archetype 必然已注册
            None => return Err(EntityError::NotFound),
        };
        let component_id = T::id();
        if T::STORAGE == ComponentStorage::Sparse {
            // Sparse 组件任意增删：列存于实体所属 Archetype，按实体槽位索引
            let column = storage
                .columns
                .entry(component_id)
                .or_insert_with(|| Box::new(SparseSet::<T>::new()));
            // u32 → usize 为扩宽转换（64 位平台无损），`as usize` 非缩窄
            let slot = e.slot().0 as usize;
            match column.as_any_mut().downcast_mut::<SparseSet<T>>() {
                Some(set) => {
                    set.insert(slot, c);
                    Ok(())
                }
                // 不可达：同一 ComponentId 对应同一类型；全局注册表保证唯一
                None => Err(EntityError::ComponentMismatch),
            }
        } else {
            // SoA 组件：受 archetype 固定组件集约束（R3.3）
            if !storage.def.has_component(component_id) {
                return Err(EntityError::ComponentMismatch);
            }
            let slots_len = storage.slots.len();
            let column = storage.columns.entry(component_id).or_insert_with(|| {
                // 惰性建列：以默认值补齐到与 slots 对齐
                Box::new(SoAColumn::<T>::with_defaults(slots_len))
            });
            match column.as_any_mut().downcast_mut::<SoAColumn<T>>() {
                Some(col) => {
                    col.set(idx, c);
                    Ok(())
                }
                // 不可达：同一 ComponentId 对应同一类型；全局注册表保证唯一
                None => Err(EntityError::ComponentMismatch),
            }
        }
    }

    /// 移除并返回组件值。
    ///
    /// - **SoA 组件**：语义为"重置默认"——取走旧值并将槽位置 `T::default()`
    ///   （不 `swap_remove`，避免破坏列与 `slots` 的紧凑对齐；元素总数不变）。
    /// - **Sparse 组件**：从 SparseSet 中移除该槽位值（O(1)）。
    ///
    /// 列不存在（从未 insert）或实体无效时返回 `None`。
    pub fn remove<T: Component>(&mut self, e: Entity) -> Option<T> {
        if !self.entities.is_alive(e) {
            return None;
        }
        let (arch, idx) = *self.entity_index.get(&e)?;
        let storage = self.archetypes.get_mut(&arch)?;
        let column = storage.columns.get_mut(&T::id())?;
        if T::STORAGE == ComponentStorage::Sparse {
            let boxed = column.take_slot(e.slot().0)?;
            boxed.downcast::<T>().ok().map(|value| *value)
        } else {
            let boxed = column.take_at(idx)?;
            boxed.downcast::<T>().ok().map(|value| *value)
        }
    }

    /// 只读获取组件引用；实体无效或组件不存在（列未创建）返回 `None`。
    pub fn get<T: Component>(&self, e: Entity) -> Option<&T> {
        if !self.entities.is_alive(e) {
            return None;
        }
        let (arch, idx) = *self.entity_index.get(&e)?;
        let storage = self.archetypes.get(&arch)?;
        let column = storage.columns.get(&T::id())?;
        if T::STORAGE == ComponentStorage::Sparse {
            // u32 → usize 为扩宽转换（64 位平台无损），`as usize` 非缩窄
            let slot = e.slot().0 as usize;
            column.as_any().downcast_ref::<SparseSet<T>>()?.get(slot)
        } else {
            column.as_any().downcast_ref::<SoAColumn<T>>()?.get(idx)
        }
    }

    /// 可变获取组件引用；实体无效或组件不存在（列未创建）返回 `None`。
    pub fn get_mut<T: Component>(&mut self, e: Entity) -> Option<&mut T> {
        if !self.entities.is_alive(e) {
            return None;
        }
        let (arch, idx) = *self.entity_index.get(&e)?;
        let storage = self.archetypes.get_mut(&arch)?;
        let column = storage.columns.get_mut(&T::id())?;
        if T::STORAGE == ComponentStorage::Sparse {
            let slot = e.slot().0 as usize;
            column
                .as_any_mut()
                .downcast_mut::<SparseSet<T>>()?
                .get_mut(slot)
        } else {
            column
                .as_any_mut()
                .downcast_mut::<SoAColumn<T>>()?
                .get_mut(idx)
        }
    }

    /// 存活实体总数（R13.3 status 数据源）。
    pub fn entity_count(&self) -> usize {
        self.entity_index.len()
    }

    /// 按实体类型统计存活实体数（键为类型 ID 升序）。
    pub fn entities_by_kind(&self) -> Vec<(EntityTypeId, usize)> {
        let mut counts: HashMap<EntityTypeId, usize> = HashMap::new();
        for entity in self.entity_index.keys() {
            *counts.entry(entity.type_id()).or_insert(0) += 1;
        }
        // EntityTypeId 未实现 Ord，按内部 u8 排序保证输出确定性
        let mut result: Vec<(EntityTypeId, usize)> = counts.into_iter().collect();
        result.sort_by_key(|(kind, _)| kind.0);
        result
    }

    /// 全部 Archetype 的已分配内存统计：实体槽位容量 + 组件列容量之和
    /// （调试/内存统计，R13.2 数据源）。
    pub fn component_capacity(&self) -> usize {
        self.archetypes
            .values()
            .map(|storage| {
                storage.slots.capacity()
                    + storage
                        .columns
                        .values()
                        .map(|column| column.capacity())
                        .sum::<usize>()
            })
            .sum()
    }

    /// 预分配：为指定 Archetype 预留 `capacity` 个实体的槽位与实体列表容量
    /// （R3.4，几何增长之外的应用级预分配）。
    ///
    /// # Panics
    ///
    /// `arch` 未注册时 panic（与 `spawn` 一致）。
    pub fn reserve_entities(&mut self, arch: ArchetypeId, capacity: usize) {
        let storage = match self.archetypes.get_mut(&arch) {
            Some(storage) => storage,
            None => panic!("未注册 Archetype：ArchetypeId({})", arch.0),
        };
        let kind = storage.def.entity_kind;
        storage.slots.reserve(capacity);
        self.entities.reserve(kind, capacity);
    }

    /// 用默认值初始化资源；已存在时保留现值（不覆盖）。
    pub fn init_resource<T: Resource + Default>(&mut self) -> &mut Self {
        if !self.resources.contains::<T>() {
            self.resources.insert(T::default());
        }
        self
    }

    /// 插入资源；同类型已存在时替换旧值。
    pub fn insert_resource<T: Resource>(&mut self, r: T) -> &mut Self {
        let _ = self.resources.insert(r);
        self
    }

    /// 移除并返回资源；未注册返回 None。
    pub fn remove_resource<T: Resource>(&mut self) -> Option<T> {
        self.resources.remove()
    }

    /// 只读获取资源引用；未注册返回 None。
    pub fn resource<T: Resource>(&self) -> Option<&T> {
        self.resources.get()
    }

    /// [`resource`] 的 旧 ECS 方案 风格别名（T17 测试迁移兼容）。
    pub fn get_resource<T: Resource>(&self) -> Option<&T> {
        self.resources.get()
    }

    /// 可变获取资源引用；未注册返回 None。
    pub fn resource_mut<T: Resource>(&mut self) -> Option<&mut T> {
        self.resources.get_mut()
    }

    /// [`resource_mut`] 的 旧 ECS 方案 风格别名（T17 测试迁移兼容）。
    pub fn get_resource_mut<T: Resource>(&mut self) -> Option<&mut T> {
        self.resources.get_mut()
    }

    /// 向对应消息收件箱写入一条消息（旧 ECS 方案 `World::write` / `send_event` 语义）。
    ///
    /// 若该消息类型尚未经 `App::add_message::<T>()` 注册（inbox 资源未预建），
    /// 消息被静默丢弃且不 panic（符合宪章生产代码零 panic 契约）。建议调用方在
    /// 构建阶段完成注册，以确保消息可靠投递。
    pub fn write<T: Message>(&mut self, msg: T) {
        if let Some(inbox) = self.resource_mut::<MessageInbox<T>>() {
            inbox.write(msg);
        }
    }

    /// 是否已注册该类型资源。
    pub fn contains_resource<T: Resource>(&self) -> bool {
        self.resources.contains::<T>()
    }

    /// 实体所属 Archetype（T8 迁移查询用，IC-12）。
    ///
    /// 未 spawn/已销毁/悬挂句柄返回 `None`。
    pub fn entity_archetype(&self, e: Entity) -> Option<ArchetypeId> {
        self.entity_index.get(&e).map(|&(arch, _)| arch)
    }

    /// 按指定 `Entity`（尽量保留 ID）在 Archetype 中落位新实体。
    ///
    /// 目标世界该实体的槽位空闲（未占用，含越界槽位）则原样保留 ID（世代与
    /// 槽位均不变，跨世界迁移 ID 不变语义，IC-12）；槽位被占用则重新分配新
    /// ID（返回实际 `Entity`，**目标槽位冲突时 ID 变化**，调用方以返回值
    /// 为准）。与 [`World::spawn`] 相同，既有 SoA 列逐列 push 默认占位保持
    /// 与 `slots` 对齐；Sparse 列按槽位索引，不预占。
    ///
    /// # Panics
    ///
    /// `arch` 未注册时 panic（与 [`World::spawn`] 一致）。
    pub fn spawn_exact(&mut self, arch: ArchetypeId, e: Entity) -> Entity {
        let kind = match self.archetypes.get(&arch) {
            Some(storage) => storage.def.entity_kind,
            None => panic!("未注册 Archetype：ArchetypeId({})", arch.0),
        };
        // 优先按原 ID 落位（allocate_exact 占用空闲槽位）；冲突则重新分配
        let entity = match self.entities.allocate_exact(kind, e) {
            Some(placed) => placed,
            None => self.entities.allocate(kind),
        };
        let storage = match self.archetypes.get_mut(&arch) {
            Some(storage) => storage,
            // 不可达：上面已验证注册
            None => panic!("未注册 Archetype：ArchetypeId({})", arch.0),
        };
        let idx = storage.slots.len();
        // 既有 SoA 列逐列 push 默认占位，保持列长度 == slots 长度
        for column in storage.columns.values_mut() {
            column.push_default();
        }
        storage.slots.push(entity);
        self.entity_index.insert(entity, (arch, idx));
        entity
    }

    /// 类型擦除组件写入（T8 迁移用）：按 `ComponentId` 定位既有列，
    /// `downcast` 还原具体类型后写入（[`crate::storage::ErasedColumn::insert_at`]）。
    ///
    /// # Errors
    ///
    /// - 实体无效（未 spawn/已销毁/悬挂）→ [`EntityError::NotFound`]。
    /// - 列不存在（目标 Archetype 从未对该组件执行过 `insert<T>`）或类型不
    ///   匹配 → [`EntityError::ComponentMismatch`]。SoA 列要求索引在界
    ///   （`spawn_exact`/`spawn` 已保证对齐，越界视为框架 bug）。
    pub fn insert_any(
        &mut self,
        e: Entity,
        cid: ComponentId,
        value: Box<dyn Any + Send + Sync>,
    ) -> Result<(), EntityError> {
        if !self.entities.is_alive(e) {
            return Err(EntityError::NotFound);
        }
        let (arch, idx) = match self.entity_index.get(&e) {
            Some(&entry) => entry,
            None => return Err(EntityError::NotFound),
        };
        let storage = match self.archetypes.get_mut(&arch) {
            Some(storage) => storage,
            // 不可达：entity_index 引用的 archetype 必然已注册
            None => return Err(EntityError::NotFound),
        };
        let column = match storage.columns.get_mut(&cid) {
            Some(column) => column,
            // 列缺失：migrate_entity 前置条件要求目标列已就绪（先 insert 过）
            None => return Err(EntityError::ComponentMismatch),
        };
        // SoA 列按 Archetype 槽位索引写入；Sparse 列按实体槽位写入
        let ok = if column.is_sparse() {
            column.insert_at(e.slot().0 as usize, value)
        } else {
            column.insert_at(idx, value)
        };
        if ok {
            Ok(())
        } else {
            Err(EntityError::ComponentMismatch)
        }
    }
}

impl Default for World {
    fn default() -> Self {
        World::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::ComponentId;
    use crate::entity::{Generation, Slot};

    #[derive(Default)]
    struct TestRes(u32);

    struct UnusedRes;

    // ---- 测试组件（手工实现 Component，避免测试依赖宏 crate）----

    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    struct Position {
        x: f32,
        y: f32,
    }

    impl Component for Position {
        fn id() -> ComponentId {
            ComponentId(1)
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
            ComponentId(2)
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
            ComponentId(3)
        }
        const STORAGE: ComponentStorage = ComponentStorage::Sparse;
        type Registry = ();
    }

    // ---- 测试 Archetype 定义（'static，可直接注册）----

    /// 玩家：SoA 组件 Position + Velocity，实体类型 1。
    static PLAYER_DEF: ArchetypeDef = ArchetypeDef {
        id: ArchetypeId(0),
        name: "PlayerArchetype",
        component_ids: &[ComponentId(1), ComponentId(2)],
        entity_kind: EntityTypeId(1),
        component_types: &[],
    };

    /// 怪物：SoA 组件 Position + Sparse 组件 Health，实体类型 2。
    static MONSTER_DEF: ArchetypeDef = ArchetypeDef {
        id: ArchetypeId(1),
        name: "MonsterArchetype",
        component_ids: &[ComponentId(1), ComponentId(3)],
        entity_kind: EntityTypeId(2),
        component_types: &[],
    };

    #[test]
    fn world_resource_lifecycle() {
        let mut world = World::new();
        assert!(!world.contains_resource::<TestRes>());
        assert!(world.resource::<TestRes>().is_none());
        assert!(world.resource_mut::<TestRes>().is_none());

        world.insert_resource(TestRes(7));
        assert!(world.contains_resource::<TestRes>());
        assert_eq!(world.resource::<TestRes>().map(|r| r.0), Some(7));

        // resource_mut 原地修改
        if let Some(res) = world.resource_mut::<TestRes>() {
            res.0 = 42;
        }
        assert_eq!(world.resource::<TestRes>().map(|r| r.0), Some(42));

        // 覆盖替换旧值
        world.insert_resource(TestRes(9));
        assert_eq!(world.resource::<TestRes>().map(|r| r.0), Some(9));

        // 移除：先 Some 后 None
        assert_eq!(world.remove_resource::<TestRes>().map(|r| r.0), Some(9));
        assert!(world.remove_resource::<TestRes>().is_none());
        assert!(!world.contains_resource::<TestRes>());
        // 未注册类型查询恒 None
        assert!(world.resource::<UnusedRes>().is_none());
    }

    #[test]
    fn world_init_resource_inserts_default() {
        let mut world = World::new();
        world.init_resource::<TestRes>();
        assert_eq!(world.resource::<TestRes>().map(|r| r.0), Some(0));
    }

    #[test]
    fn world_init_resource_does_not_overwrite() {
        let mut world = World::new();
        world.insert_resource(TestRes(7));
        world.init_resource::<TestRes>();
        assert_eq!(world.resource::<TestRes>().map(|r| r.0), Some(7));
    }

    #[test]
    fn world_default_is_empty() {
        let world = World::default();
        assert!(!world.contains_resource::<TestRes>());
        assert_eq!(world.entity_count(), 0);
    }

    // ---- T3 实体 CRUD ----

    #[test]
    fn spawn_contains_and_count() {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        world.register_archetype(&MONSTER_DEF);
        let p1 = world.spawn(ArchetypeId(0));
        let p2 = world.spawn(ArchetypeId(0));
        let m1 = world.spawn(ArchetypeId(1));
        assert!(world.contains(p1));
        assert!(world.contains(p2));
        assert!(world.contains(m1));
        assert_eq!(world.entity_count(), 3);
        // 未 spawn 的悬挂句柄不存活
        let ghost = Entity::from_parts(EntityTypeId(1), Generation(0), Slot(999));
        assert!(!world.contains(ghost));
        // 玩家类型实体槽位独立于怪物类型（R1.4）
        assert_eq!(p1.type_id(), EntityTypeId(1));
        assert_eq!(m1.type_id(), EntityTypeId(2));
    }

    #[test]
    #[should_panic]
    fn spawn_unregistered_archetype_panics() {
        let mut world = World::new();
        let _ = world.spawn(ArchetypeId(99));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic]
    fn register_archetype_duplicate_panics() {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        // debug 构建下重复注册断言失败
        world.register_archetype(&PLAYER_DEF);
    }

    #[test]
    fn insert_soa_creates_column_and_updates_value() {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        let e1 = world.spawn(ArchetypeId(0));
        let e2 = world.spawn(ArchetypeId(0));
        // 未 insert 前列未创建：get 返回 None（惰性列语义）
        assert!(world.get::<Position>(e1).is_none());
        // 首次 insert 创建列：中间实体 e2 获得默认值（对齐补齐）
        assert!(world.insert(e1, Position { x: 1.0, y: 2.0 }).is_ok());
        assert_eq!(
            world.get::<Position>(e1),
            Some(&Position { x: 1.0, y: 2.0 })
        );
        assert_eq!(world.get::<Position>(e2), Some(&Position::default()));
        // get_mut 原地更新
        world.get_mut::<Position>(e2).unwrap().x = 5.0;
        assert_eq!(world.get::<Position>(e2).map(|p| p.x), Some(5.0));
        // 覆盖更新既有实体
        assert!(world.insert(e1, Position { x: 9.0, y: 9.0 }).is_ok());
        assert_eq!(world.get::<Position>(e1).map(|p| p.x), Some(9.0));
    }

    #[test]
    fn insert_soa_second_component_shares_column_alignment() {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        let e1 = world.spawn(ArchetypeId(0));
        let e2 = world.spawn(ArchetypeId(0));
        let _ = world.insert(e1, Position { x: 1.0, y: 1.0 });
        // spawn 发生在 Position 列创建之后：新实体 push 默认占位，列保持对齐
        let e3 = world.spawn(ArchetypeId(0));
        assert_eq!(world.get::<Position>(e3), Some(&Position::default()));
        // Velocity 列此时创建：补齐全部 3 个实体
        let _ = world.insert(e2, Velocity { dx: 2.0, dy: 0.0 });
        assert_eq!(world.get::<Velocity>(e1), Some(&Velocity::default()));
        assert_eq!(world.get::<Velocity>(e2).map(|v| v.dx), Some(2.0));
        assert_eq!(world.get::<Velocity>(e3), Some(&Velocity::default()));
    }

    #[test]
    fn insert_soa_mismatch_archetype_returns_err() {
        let mut world = World::new();
        world.register_archetype(&MONSTER_DEF); // 无 Velocity
        let e = world.spawn(ArchetypeId(1));
        // SoA 组件 Velocity 不属于 Monster 固定组件集
        assert_eq!(
            world.insert(e, Velocity { dx: 1.0, dy: 0.0 }),
            Err(EntityError::ComponentMismatch)
        );
        // 失败路径不创建列：get 恒 None
        assert!(world.get::<Velocity>(e).is_none());
    }

    #[test]
    fn insert_sparse_any_archetype() {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF); // 固定组件集不含 Health
        let e = world.spawn(ArchetypeId(0));
        // Sparse 组件任意增删，不受 archetype 组件集约束（R3.3）
        assert!(world.insert(e, Health { hp: 20 }).is_ok());
        assert_eq!(world.get::<Health>(e), Some(&Health { hp: 20 }));
        assert_eq!(world.remove::<Health>(e), Some(Health { hp: 20 }));
        assert!(world.get::<Health>(e).is_none());
    }

    #[test]
    fn sparse_columns_isolated_between_entities() {
        let mut world = World::new();
        world.register_archetype(&MONSTER_DEF);
        let m1 = world.spawn(ArchetypeId(1));
        let m2 = world.spawn(ArchetypeId(1));
        let _ = world.insert(m1, Health { hp: 100 });
        let _ = world.insert(m2, Health { hp: 200 });
        assert_eq!(world.get::<Health>(m1), Some(&Health { hp: 100 }));
        assert_eq!(world.get::<Health>(m2), Some(&Health { hp: 200 }));
        // Sparse 组件移除不影响其他实体
        assert_eq!(world.remove::<Health>(m1), Some(Health { hp: 100 }));
        assert_eq!(world.get::<Health>(m2), Some(&Health { hp: 200 }));
    }

    #[test]
    fn remove_soa_resets_to_default() {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        let e = world.spawn(ArchetypeId(0));
        let _ = world.insert(e, Position { x: 3.0, y: 4.0 });
        // SoA remove：返回旧值，槽位重置默认（不破坏对齐）
        assert_eq!(
            world.remove::<Position>(e),
            Some(Position { x: 3.0, y: 4.0 })
        );
        assert_eq!(world.get::<Position>(e), Some(&Position::default()));
        assert_eq!(world.entity_count(), 1);
        // 从未 insert 的组件 remove 返回 None
        assert_eq!(world.remove::<Velocity>(e), None);
    }

    #[test]
    fn despawn_removes_entity_and_components() {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        let e1 = world.spawn(ArchetypeId(0));
        let e2 = world.spawn(ArchetypeId(0));
        let _ = world.insert(e1, Position { x: 1.0, y: 0.0 });
        let _ = world.insert(e2, Position { x: 2.0, y: 0.0 });
        let _ = world.insert(e1, Health { hp: 5 }); // Sparse 组件随实体清理
        assert!(world.despawn(e1));
        assert!(!world.contains(e1));
        assert!(world.get::<Position>(e1).is_none());
        assert!(world.get::<Health>(e1).is_none());
        // swap_remove 后 e2 移到索引 0，entity_index 已同步：数据不丢失
        assert!(world.contains(e2));
        assert_eq!(world.get::<Position>(e2).map(|p| p.x), Some(2.0));
        assert_eq!(world.entity_count(), 1);
        // 二次 despawn 返回 false
        assert!(!world.despawn(e1));
        // 悬挂句柄 despawn 返回 false
        let ghost = Entity::from_parts(EntityTypeId(1), Generation(0), Slot(999));
        assert!(!world.despawn(ghost));
        // 全部销毁后计数归零
        assert!(world.despawn(e2));
        assert_eq!(world.entity_count(), 0);
    }

    #[test]
    fn despawn_last_entity_no_index_shift() {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        let e1 = world.spawn(ArchetypeId(0));
        let e2 = world.spawn(ArchetypeId(0));
        let _ = world.insert(e1, Position { x: 1.0, y: 0.0 });
        let _ = world.insert(e2, Position { x: 2.0, y: 0.0 });
        // 销毁末尾实体：无 swap 交换，entity_index 无需更新
        assert!(world.despawn(e2));
        assert!(world.contains(e1));
        assert_eq!(world.get::<Position>(e1).map(|p| p.x), Some(1.0));
        assert_eq!(world.entity_count(), 1);
    }

    #[test]
    fn entities_by_kind_counts() {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        world.register_archetype(&MONSTER_DEF);
        let _ = world.spawn(ArchetypeId(0));
        let _ = world.spawn(ArchetypeId(0));
        let _ = world.spawn(ArchetypeId(1));
        assert_eq!(
            world.entities_by_kind(),
            vec![(EntityTypeId(1), 2), (EntityTypeId(2), 1)]
        );
        // 销毁后计数随之更新
        let e = world.spawn(ArchetypeId(1));
        assert!(world.despawn(e));
        assert_eq!(
            world.entities_by_kind(),
            vec![(EntityTypeId(1), 2), (EntityTypeId(2), 1)]
        );
    }

    #[test]
    fn component_capacity_and_reserve_entities() {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        // 预分配：slots 容量预留
        world.reserve_entities(ArchetypeId(0), 64);
        assert!(world.component_capacity() >= 64);
        // 预分配后 spawn 不触发扩容
        let mut entities = Vec::new();
        for _ in 0..64 {
            entities.push(world.spawn(ArchetypeId(0)));
        }
        assert_eq!(world.entity_count(), 64);
        assert!(world.component_capacity() >= 64);
        // 组件列创建后容量计入统计
        for e in entities.iter().take(4) {
            let _ = world.insert(*e, Position { x: 1.0, y: 1.0 });
        }
        assert!(world.component_capacity() > 64);
    }

    #[test]
    #[should_panic]
    fn reserve_entities_unregistered_archetype_panics() {
        let mut world = World::new();
        // 未注册 archetype：与 spawn 一致的 panic 语义
        world.reserve_entities(ArchetypeId(99), 16);
    }

    #[test]
    fn entity_error_variants_covered() {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        world.register_archetype(&MONSTER_DEF);
        let e = world.spawn(ArchetypeId(0));
        // NotFound：实体不存在（悬挂句柄）
        let ghost = Entity::from_parts(EntityTypeId(1), Generation(0), Slot(999));
        assert_eq!(
            world.insert(ghost, Position::default()),
            Err(EntityError::NotFound)
        );
        // NotFound：销毁后操作
        assert!(world.despawn(e));
        assert_eq!(
            world.insert(e, Position::default()),
            Err(EntityError::NotFound)
        );
        assert!(world.remove::<Position>(e).is_none());
        assert!(world.get::<Position>(e).is_none());
        // ComponentMismatch：SoA 组件不属于该 archetype（Monster 固定组件集
        // 无 Velocity）
        let monster = world.spawn(ArchetypeId(1));
        assert_eq!(
            world.insert(monster, Velocity { dx: 1.0, dy: 1.0 }),
            Err(EntityError::ComponentMismatch)
        );
        // Despawned：文档性变体，直接构造验证可匹配/比较
        assert_eq!(EntityError::Despawned, EntityError::Despawned);
        let _ = matches!(EntityError::Despawned, EntityError::Despawned);
    }

    #[test]
    fn world_registered_archetypes_storage() {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        world.register_archetype(&MONSTER_DEF);
        let storage = world.archetypes.get(&ArchetypeId(0)).unwrap();
        assert_eq!(storage.def.name, "PlayerArchetype");
        assert_eq!(storage.slots.len(), 0);
        assert!(storage.columns.is_empty());
    }

    // ---- T8 迁移辅助方法 ----

    #[test]
    fn entity_archetype_queries() {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        world.register_archetype(&MONSTER_DEF);
        let p = world.spawn(ArchetypeId(0));
        let m = world.spawn(ArchetypeId(1));
        assert_eq!(world.entity_archetype(p), Some(ArchetypeId(0)));
        assert_eq!(world.entity_archetype(m), Some(ArchetypeId(1)));
        // 悬挂句柄返回 None
        let ghost = Entity::from_parts(EntityTypeId(1), Generation(0), Slot(999));
        assert_eq!(world.entity_archetype(ghost), None);
        // despawn 后返回 None
        assert!(world.despawn(p));
        assert_eq!(world.entity_archetype(p), None);
    }

    #[test]
    fn spawn_exact_preserves_id_when_slot_free() {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        // 从未分配过的槽位 0：按原 ID 落位（世代/槽位均保留）
        let e = Entity::from_parts(EntityTypeId(1), Generation(0), Slot(0));
        let placed = world.spawn_exact(ArchetypeId(0), e);
        assert_eq!(placed, e);
        assert!(world.contains(placed));
        assert_eq!(world.entity_count(), 1);
        // 槽位被占：重新分配新 ID（返回值与请求不同）
        let second = world.spawn_exact(ArchetypeId(0), e);
        assert_ne!(second, e);
        assert!(world.contains(second));
        assert_eq!(world.entity_count(), 2);
        // 列对齐：既有列随落位 push 默认占位
        let _ = world.insert(e, Position { x: 1.0, y: 2.0 });
        assert_eq!(world.get::<Position>(second), Some(&Position::default()));
    }

    #[test]
    fn spawn_exact_reuses_freed_slot_with_original_generation() {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        let old = world.spawn(ArchetypeId(0)); // 槽位 0，世代 0
        assert!(world.despawn(old)); // 槽位 0 空闲（free 栈）
        // 以更高世代落位到已释放槽位：ID 原样保留
        let e = Entity::from_parts(EntityTypeId(1), Generation(5), Slot(0));
        let placed = world.spawn_exact(ArchetypeId(0), e);
        assert_eq!(placed, e);
        assert!(world.contains(placed));
        // 旧句柄世代不匹配，已失效
        assert!(!world.contains(old));
    }

    #[test]
    fn insert_any_writes_existing_columns() {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        let e = world.spawn(ArchetypeId(0));
        // 建列（惰性）：Position SoA + Health Sparse
        let _ = world.insert(e, Position { x: 1.0, y: 2.0 });
        let _ = world.insert(e, Health { hp: 7 });
        // SoA 列类型擦除写入
        let boxed: Box<dyn std::any::Any + Send + Sync> = Box::new(Position { x: 9.0, y: 8.0 });
        assert!(world.insert_any(e, Position::id(), boxed).is_ok());
        assert_eq!(world.get::<Position>(e), Some(&Position { x: 9.0, y: 8.0 }));
        // Sparse 列类型擦除写入
        let boxed: Box<dyn std::any::Any + Send + Sync> = Box::new(Health { hp: 42 });
        assert!(world.insert_any(e, Health::id(), boxed).is_ok());
        assert_eq!(world.get::<Health>(e), Some(&Health { hp: 42 }));
    }

    #[test]
    fn insert_any_error_branches() {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        let e = world.spawn(ArchetypeId(0));
        // 类型不匹配（downcast 失败）：ComponentMismatch
        let _ = world.insert(e, Position { x: 0.0, y: 0.0 });
        let wrong: Box<dyn std::any::Any + Send + Sync> = Box::new(42u32);
        assert_eq!(
            world.insert_any(e, Position::id(), wrong),
            Err(EntityError::ComponentMismatch)
        );
        // 列缺失（该组件从未 insert）：ComponentMismatch
        let boxed: Box<dyn std::any::Any + Send + Sync> = Box::new(Position { x: 0.0, y: 0.0 });
        assert_eq!(
            world.insert_any(e, Velocity::id(), boxed),
            Err(EntityError::ComponentMismatch)
        );
        // 无效实体：NotFound
        let ghost = Entity::from_parts(EntityTypeId(1), Generation(0), Slot(999));
        let boxed: Box<dyn std::any::Any + Send + Sync> = Box::new(Position { x: 0.0, y: 0.0 });
        assert_eq!(
            world.insert_any(ghost, Position::id(), boxed),
            Err(EntityError::NotFound)
        );
    }
}
