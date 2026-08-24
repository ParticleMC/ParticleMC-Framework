//! 实体标识与按实体类型隔离的槽位分配器。
//!
//! 变更标识符：`implement-custom-ecs`
//!
//! 实体句柄为 u64 分段编码：`[类型ID 高8位 | 世代 中24位 | 槽位 低32位]`
//! （IC-1）。世代在槽位复用（deallocate 后再 allocate）时递增，使旧句柄失效，
//! 防止悬挂引用（R1.2/R1.3）。不同类型实体的槽位空间相互隔离（R1.4）：
//! [`EntityArena`] 内每实体类型一个 [`TypeArena`]。

use std::collections::HashMap;

use crate::util::next_power_of_two;

/// 实体句柄：u64 分段编码 = [类型ID 高8位 | 世代 中24位 | 槽位 低32位]。
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct Entity(pub(crate) u64);

/// 实体类型 ID：实体 ID 高 8 位，取值 0-255。
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct EntityTypeId(pub u8);

/// 实体世代：实体 ID 中 24 位，槽位复用（防悬挂）计数，最大 0xFF_FFFF。
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Generation(pub u32);

/// 槽位：实体 ID 低 32 位，指向所属类型 Arena 中的槽位索引。
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Slot(pub u32);

impl Entity {
    /// 占位实体：type/generation/slot 全零，代表「无实体」哨兵值。
    pub const PLACEHOLDER: Entity = Entity::from_parts(EntityTypeId(0), Generation(0), Slot(0));

    /// 由三段编码组装实体句柄。
    ///
    /// 位布局：`(kind << 56) | (generation << 32) | slot`。世代越界
    /// （> 0xFF_FFFF）在 debug 构建下断言（release 不 panic，契约交给调用方保证）。
    pub const fn from_parts(kind: EntityTypeId, generation: Generation, slot: Slot) -> Entity {
        debug_assert!(generation.0 <= 0xFF_FFFF, "世代越界");
        Entity(((kind.0 as u64) << 56) | ((generation.0 as u64) << 32) | (slot.0 as u64))
    }

    /// 兼容构造器（迁移期）：以槽位索引直接构造实体（type/generation 归零）。
    ///
    /// 对应 旧 ECS 方案 `Entity::from_raw` / minestom-rs `Entity::from_raw_u32` 的测试用法；
    /// 仅用于测试与占位，运行期实体应由 `World` 分配器产出。
    pub const fn from_raw_u32(slot: u32) -> Entity {
        Entity::from_parts(EntityTypeId(0), Generation(0), Slot(slot))
    }

    /// 实体类型 ID（高 8 位）。
    ///
    /// 右移 56 位后高位清零，值必在 0-255 内，`as u8` 无损。
    pub const fn type_id(&self) -> EntityTypeId {
        EntityTypeId((self.0 >> 56) as u8)
    }

    /// 世代（中 24 位）。
    ///
    /// 右移 32 位后与 0xFF_FFFF 相与清零高位，`as u32` 无损。
    pub const fn generation(&self) -> Generation {
        Generation(((self.0 >> 32) & 0xFF_FFFF) as u32)
    }

    /// 槽位（低 32 位）。
    ///
    /// 与 0xFFFF_FFFF 相与清零高位，`as u32` 无损。
    pub const fn slot(&self) -> Slot {
        Slot((self.0 & 0xFFFF_FFFF) as u32)
    }

    /// 原始 u64 编码。
    pub const fn to_bits(&self) -> u64 {
        self.0
    }

    /// 槽位索引（旧 ECS 方案 `Entity::index()` 对齐）：返回低 32 位槽位值。
    pub const fn index(&self) -> u32 {
        self.slot().0
    }

    /// 槽位索引（u32，旧 ECS 方案 `Entity::index_u32()` 对齐）：与 [`Entity::index`] 同值。
    pub const fn index_u32(&self) -> u32 {
        self.slot().0
    }
}

/// 槽位状态：世代计数与占用标记。
struct SlotState {
    generation: u32,
    occupied: bool,
}

/// 单实体类型的槽位空间：`slots` 存槽位状态，`free` 为可复用槽位的空闲栈。
///
/// 空闲栈复用：`deallocate` 将槽位索引压栈（O(1)），`allocate` 出栈并世代 +1，
/// 使所有旧句柄失效（防悬挂）。无空闲槽时按几何增长（×2）扩容（R1.3），
/// 新槽位世代从 0 开始。
struct TypeArena {
    slots: Vec<SlotState>,
    free: Vec<Slot>,
    /// 本类型存活实体数（deallocate 递减，allocate 递增）。
    len: usize,
}

/// 无预分配时的最小初始容量（几何增长的起点）。
const MIN_ARENA_SLOTS: usize = 16;

impl TypeArena {
    fn new() -> Self {
        TypeArena {
            slots: Vec::new(),
            free: Vec::new(),
            len: 0,
        }
    }

    fn allocate(&mut self, kind: EntityTypeId) -> Entity {
        // 优先复用空闲槽：世代 +1 使旧句柄失效，杜绝悬挂引用
        if let Some(slot) = self.free.pop() {
            let idx = slot.0 as usize;
            let state = match self.slots.get_mut(idx) {
                Some(state) => state,
                // free 栈仅由 deallocate 入栈（入栈前已校验槽位在界），此分支不可达
                None => unreachable!("空闲槽索引越界，与 deallocate 不变量冲突"),
            };
            state.generation = state.generation.saturating_add(1);
            state.occupied = true;
            self.len += 1;
            return Entity::from_parts(kind, Generation(state.generation), slot);
        }
        // 无空闲槽：几何增长 ×2 预留容量后新建槽位（generation = 0）
        let target = next_power_of_two((self.slots.len() * 2).max(1)).max(MIN_ARENA_SLOTS);
        if self.slots.len() < target {
            self.slots.reserve(target - self.slots.len());
        }
        let idx = self.slots.len();
        self.slots.push(SlotState {
            generation: 0,
            occupied: true,
        });
        self.len += 1;
        // 单类型槽位数超过 2^32 在物理上不可达（内存需求远超实际），饱和仅形式性兜底
        let slot = u32::try_from(idx).unwrap_or(u32::MAX);
        Entity::from_parts(kind, Generation(0), Slot(slot))
    }

    fn deallocate(&mut self, entity: Entity) -> bool {
        let idx = entity.slot().0 as usize;
        let state = match self.slots.get_mut(idx) {
            Some(state) => state,
            // 槽位越界：悬挂句柄或从未分配过的槽位
            None => return false,
        };
        if !state.occupied || state.generation != entity.generation().0 {
            // 双重释放或世代不匹配：句柄已失效，拒绝操作
            return false;
        }
        state.occupied = false;
        self.free.push(entity.slot());
        self.len -= 1;
        true
    }

    fn is_alive(&self, entity: Entity) -> bool {
        match self.slots.get(entity.slot().0 as usize) {
            Some(state) => state.occupied && state.generation == entity.generation().0,
            // 槽位越界：不存活
            None => false,
        }
    }
}

/// 按实体类型隔离的槽位分配器。
///
/// 每种实体类型一个 [`TypeArena`]（槽位空间互不共享），以 `EntityTypeId` 为键。
/// 独立类型、不直接嵌入 `World`（T3 集成）。`allocate`/`deallocate` 均 O(1)。
pub struct EntityArena {
    types: HashMap<EntityTypeId, TypeArena>,
}

impl EntityArena {
    /// 空分配器（未分配任何类型的槽位）。
    pub fn new() -> Self {
        EntityArena {
            types: HashMap::new(),
        }
    }

    /// 为指定类型分配一个实体句柄（O(1)）。
    ///
    /// 优先复用空闲槽（世代 +1），无空闲槽时扩容新建（generation = 0）。
    pub fn allocate(&mut self, kind: EntityTypeId) -> Entity {
        self.types
            .entry(kind)
            .or_insert_with(TypeArena::new)
            .allocate(kind)
    }

    /// 销毁实体并释放其槽位（O(1)），返回句柄在释放前是否仍有效。
    ///
    /// 成功释放后槽位进入空闲栈，下次 `allocate` 复用并世代 +1（R1.4）。
    pub fn deallocate(&mut self, entity: Entity) -> bool {
        match self.types.get_mut(&entity.type_id()) {
            Some(arena) => arena.deallocate(entity),
            // 该实体类型从未分配过
            None => false,
        }
    }

    /// 按指定实体精确占用其槽位（T8 跨世界迁移保留 ID 用，IC-12）。
    ///
    /// 目标槽位空闲（未占用，含越界槽位——自动扩容至目标下标）则占用并返回
    /// 该实体（世代与槽位均按 `e` 原样落位，ID 不变）；槽位已占用返回 `None`
    /// （调用方应改用 [`allocate`] 重新分配，ID 变化）。
    ///
    /// 占用时若该槽位恰在空闲复用栈中（先前 `deallocate` 入栈），同步将其从
    /// 栈中移除，避免后续 `allocate` 误复用已占用槽位。
    pub fn allocate_exact(&mut self, kind: EntityTypeId, e: Entity) -> Option<Entity> {
        // 实体类型须与目标类型一致（跨世界迁移时源实体类型由 Archetype 决定）
        if e.type_id() != kind {
            return None;
        }
        let arena = self.types.entry(kind).or_insert_with(TypeArena::new);
        let slot_u32 = e.slot().0;
        let idx = slot_u32 as usize;
        if idx >= arena.slots.len() {
            // 越界槽位视为空闲：扩容至目标下标（迁移场景各世界槽位空间独立，
            // 目标世界可能从未分配过该下标；极端槽位值的内存成本由调用方承担）
            arena.slots.resize_with(idx + 1, || SlotState {
                generation: 0,
                occupied: false,
            });
        } else if arena.slots[idx].occupied {
            // 目标槽位已占用：无法保留 ID，返回 None 让调用方重新分配
            return None;
        }
        // 槽位恰在空闲复用栈中：移除，防止后续 allocate 弹出后覆盖本占用
        if let Some(pos) = arena.free.iter().position(|&s| s.0 == slot_u32) {
            arena.free.swap_remove(pos);
        }
        let state = &mut arena.slots[idx];
        state.generation = e.generation().0;
        state.occupied = true;
        arena.len += 1;
        Some(e)
    }

    /// 句柄是否存活：槽位在界、占用且世代匹配。
    pub fn is_alive(&self, entity: Entity) -> bool {
        self.types
            .get(&entity.type_id())
            .is_some_and(|arena| arena.is_alive(entity))
    }

    /// 为指定类型预分配 n 个槽位容量（不创建槽位状态，首次 allocate 不再扩容）。
    pub fn with_capacity(&mut self, kind: EntityTypeId, n: usize) {
        self.types
            .entry(kind)
            .or_insert_with(TypeArena::new)
            .slots
            .reserve(n);
    }

    /// 为指定类型额外预留 `additional` 个槽位容量。
    pub fn reserve(&mut self, kind: EntityTypeId, additional: usize) {
        self.types
            .entry(kind)
            .or_insert_with(TypeArena::new)
            .slots
            .reserve(additional);
    }

    /// 全部类型的已分配槽位容量之和（内存统计，R13.2 数据源）。
    pub fn capacity(&self) -> usize {
        self.types
            .values()
            .map(|arena| arena.slots.capacity())
            .sum()
    }

    /// 全部类型的存活实体数之和。
    pub fn len(&self) -> usize {
        self.types.values().map(|arena| arena.len).sum()
    }

    /// 是否无存活实体。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for EntityArena {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_bit_layout_positions() {
        // 位布局：u64 = [kind 高8位 | generation 中24位 | slot 低32位]
        let e = Entity::from_parts(EntityTypeId(0xAB), Generation(0x12_3456), Slot(0x789A_BCDE));
        assert_eq!(
            e.to_bits(),
            (0xABu64 << 56) | (0x12_3456u64 << 32) | 0x789A_BCDEu64
        );
    }

    #[test]
    fn entity_bits_roundtrip() {
        // 各字段边界值：0 / 类型 0xFF / 世代 0xFF_FFFF / 槽位 0xFFFF_FFFF
        let cases = [
            (EntityTypeId(0), Generation(0), Slot(0)),
            (EntityTypeId(0xFF), Generation(0), Slot(0)),
            (EntityTypeId(0), Generation(0xFF_FFFF), Slot(0)),
            (EntityTypeId(0), Generation(0), Slot(0xFFFF_FFFF)),
            (EntityTypeId(0x42), Generation(0xDE_ADBE), Slot(0xCAFE_F00D)),
        ];
        for (kind, r#gen, slot) in cases {
            let e = Entity::from_parts(kind, r#gen, slot);
            assert_eq!(e.type_id(), kind);
            assert_eq!(e.generation(), r#gen);
            assert_eq!(e.slot(), slot);
            assert_eq!(e.to_bits(), Entity::from_parts(kind, r#gen, slot).to_bits());
        }
    }

    #[test]
    fn entity_placeholder_is_all_zero() {
        assert_eq!(Entity::PLACEHOLDER.to_bits(), 0);
        assert_eq!(Entity::PLACEHOLDER.type_id(), EntityTypeId(0));
        assert_eq!(Entity::PLACEHOLDER.generation(), Generation(0));
        assert_eq!(Entity::PLACEHOLDER.slot(), Slot(0));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic]
    fn from_parts_rejects_oversized_generation() {
        // 世代 0x1_000000 超出 24 位可表示范围，debug 构建必须断言失败
        let _ = Entity::from_parts(EntityTypeId(1), Generation(0x1_000000), Slot(0));
    }

    #[test]
    fn arena_allocates_sequential_slots() {
        let mut arena = EntityArena::new();
        let a = arena.allocate(EntityTypeId(1));
        let b = arena.allocate(EntityTypeId(1));
        assert_eq!(a.slot().0, 0);
        assert_eq!(b.slot().0, 1);
        assert_eq!(a.generation().0, 0);
        assert_eq!(b.generation().0, 0);
        assert_eq!(arena.len(), 2);
        assert!(arena.is_alive(a));
        assert!(arena.is_alive(b));
    }

    #[test]
    fn arena_reuses_freed_slot_with_bumped_generation() {
        let mut arena = EntityArena::new();
        let a = arena.allocate(EntityTypeId(1));
        assert!(arena.deallocate(a));
        assert!(!arena.is_alive(a));
        let b = arena.allocate(EntityTypeId(1));
        assert_eq!(b.slot(), a.slot());
        assert_eq!(b.generation().0, a.generation().0 + 1);
        assert_ne!(b, a);
        assert!(arena.is_alive(b));
        // 旧句柄世代不匹配，失效
        assert!(!arena.is_alive(a));
        assert_eq!(arena.len(), 1);
    }

    #[test]
    fn arena_deallocate_invalid_handles() {
        let mut arena = EntityArena::new();
        let e = arena.allocate(EntityTypeId(1));
        assert!(arena.deallocate(e));
        // 双重释放
        assert!(!arena.deallocate(e));
        // 世代不匹配的旧句柄
        let stale = Entity::from_parts(EntityTypeId(1), Generation(0), Slot(0));
        assert!(!arena.deallocate(stale));
        // 槽位越界的幽灵句柄
        assert!(!arena.deallocate(Entity::from_parts(
            EntityTypeId(1),
            Generation(0),
            Slot(999)
        )));
        // 从未注册的类型
        assert!(!arena.deallocate(Entity::from_parts(EntityTypeId(9), Generation(0), Slot(0))));
    }

    #[test]
    fn arena_isolates_slots_per_kind() {
        let mut arena = EntityArena::new();
        let player = arena.allocate(EntityTypeId(1));
        let monster = arena.allocate(EntityTypeId(2));
        // 跨类型槽位空间独立：各自从 0 开始
        assert_eq!(player.slot().0, 0);
        assert_eq!(monster.slot().0, 0);
        assert_ne!(player.type_id(), monster.type_id());
        assert_ne!(player, monster);
        assert!(arena.is_alive(player));
        assert!(arena.is_alive(monster));
        // 释放玩家槽位不影响怪物类型
        assert!(arena.deallocate(player));
        assert!(arena.is_alive(monster));
        let player2 = arena.allocate(EntityTypeId(1));
        assert_eq!(player2.slot(), player.slot());
        assert_eq!(player2.generation().0, 1);
        assert!(arena.is_alive(monster));
        assert_eq!(arena.len(), 2);
    }

    #[test]
    fn arena_preallocation_capacity() {
        let mut arena = EntityArena::new();
        arena.with_capacity(EntityTypeId(1), 64);
        assert!(arena.capacity() >= 64);
        assert_eq!(arena.len(), 0);
        // 预分配后连续 allocate 不触发扩容
        for _ in 0..64 {
            arena.allocate(EntityTypeId(1));
        }
        assert_eq!(arena.len(), 64);
        assert!(arena.capacity() >= 64);
        // reserve 在已用容量之上追加预留（Vec::reserve 保证 capacity >= len + additional）
        arena.reserve(EntityTypeId(1), 32);
        assert!(arena.capacity() >= 96);
    }

    #[test]
    fn arena_grows_geometrically() {
        let mut arena = EntityArena::new();
        assert_eq!(arena.capacity(), 0);
        let n = 20; // 超过 MIN_ARENA_SLOTS(16)，触发一次几何扩容
        let mut last_slot = 0;
        for _ in 0..n {
            let e = arena.allocate(EntityTypeId(3));
            last_slot = e.slot().0;
        }
        assert_eq!(last_slot, (n - 1) as u32);
        assert_eq!(arena.len(), n);
        // 容量按 ×2 增长：16 → 32
        assert!(arena.capacity() >= 32);
    }

    #[test]
    fn arena_is_alive_checks() {
        let mut arena = EntityArena::new();
        let e = arena.allocate(EntityTypeId(1));
        assert!(arena.is_alive(e));
        // 槽位越界
        assert!(!arena.is_alive(Entity::from_parts(
            EntityTypeId(1),
            Generation(0),
            Slot(999)
        )));
        // 类型未注册
        assert!(!arena.is_alive(Entity::from_parts(
            EntityTypeId(200),
            Generation(0),
            Slot(0)
        )));
        // 已销毁
        assert!(arena.deallocate(e));
        assert!(!arena.is_alive(e));
    }

    #[test]
    fn allocate_exact_occupies_free_slot_preserving_id() {
        let mut arena = EntityArena::new();
        // 从未分配过的越界槽位：视为空闲，自动扩容并按原 ID 落位
        let e = Entity::from_parts(EntityTypeId(1), Generation(3), Slot(10));
        assert_eq!(arena.allocate_exact(EntityTypeId(1), e), Some(e));
        assert!(arena.is_alive(e));
        assert_eq!(arena.len(), 1);
        // 世代与槽位均保留：ID 不变
        assert_eq!(arena.allocate_exact(EntityTypeId(1), e), None);
    }

    #[test]
    fn allocate_exact_rejects_occupied_slot() {
        let mut arena = EntityArena::new();
        let occupied = arena.allocate(EntityTypeId(1)); // 槽位 0
        // 同类型同槽位：被占 → None
        let conflict = Entity::from_parts(EntityTypeId(1), Generation(0), Slot(0));
        assert_eq!(arena.allocate_exact(EntityTypeId(1), conflict), None);
        assert!(arena.is_alive(occupied));
        // 类型不匹配：None
        let other_kind = Entity::from_parts(EntityTypeId(2), Generation(0), Slot(0));
        assert_eq!(arena.allocate_exact(EntityTypeId(1), other_kind), None);
    }

    #[test]
    fn allocate_exact_after_deallocate_removes_from_free_stack() {
        let mut arena = EntityArena::new();
        let a = arena.allocate(EntityTypeId(1)); // 槽位 0
        let b = arena.allocate(EntityTypeId(1)); // 槽位 1
        assert!(arena.deallocate(a)); // 槽位 0 进入空闲栈
        // 精确占用已释放槽位 0（世代重置为待迁移实体的世代）
        let migrated = Entity::from_parts(EntityTypeId(1), Generation(7), Slot(0));
        assert_eq!(
            arena.allocate_exact(EntityTypeId(1), migrated),
            Some(migrated)
        );
        // 空闲栈已移除槽位 0：下次 allocate 不会复用/覆盖迁移实体，
        // 而是扩容新建槽位 2（槽位 1 仍被 b 占用）
        let c = arena.allocate(EntityTypeId(1));
        assert_eq!(c.slot().0, 2);
        assert_eq!(c.generation().0, 0);
        // 迁移实体与后续分配共存，互不覆盖
        assert!(arena.is_alive(migrated));
        assert!(arena.is_alive(b));
        assert!(arena.is_alive(c));
        assert_eq!(arena.len(), 3);
    }
}
