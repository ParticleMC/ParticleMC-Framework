// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! 热路径零堆分配验证（R3.4 / Scenario: 零分配验证）。
//!
//! 变更标识符：`implement-custom-ecs`
//!
//! 本文件为独立集成测试二进制：自定义 `#[global_allocator]` 计数（不干扰
//! crate 内部单元测试的分配行为）。流程：预分配 + 首次 spawn + 首次 insert
//! （允许扩容分配）之后，循环执行热路径（get_mut 更新 + 组件读取列迭代 +
//! 列容量迭代），断言**分配计数保持不变（0 次）**。

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use particlemc_framework_ecs::archetype::{ArchetypeDef, ArchetypeId};
use particlemc_framework_ecs::component::{Component, ComponentId, ComponentStorage};
use particlemc_framework_ecs::entity::EntityTypeId;
use particlemc_framework_ecs::storage::soa::SoAColumn;
use particlemc_framework_ecs::world::World;

/// 分配计数分配器：只统计分配/重分配次数（字节数无关），供零分配断言。
struct CountingAllocator;

/// 全局分配计数（跨线程原子，本二进制内唯一）。
static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

/// 串行化锁：本二进制内两个测试共享全局 `ALLOC_COUNT`，必须互斥执行以避免
/// 一方热路径的计数被另一方构造期分配污染（全局分配器为进程级唯一，无法按
/// 测试隔离）。每个测试入口持锁至结束，保证计数器窗口互不重叠。
static TEST_LOCK: Mutex<()> = Mutex::new(());

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        // SAFETY: 先决条件由 GlobalAlloc 契约保证，直接委托标准分配器
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: 委托标准分配器
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        // SAFETY: 委托标准分配器
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL_ALLOCATOR: CountingAllocator = CountingAllocator;

/// 测试热组件：SoA 列（与真实引擎 Position 同形态）。
#[derive(Clone, Copy, Debug, PartialEq)]
struct Position {
    x: f32,
    y: f32,
}

impl Default for Position {
    fn default() -> Self {
        Position { x: 0.0, y: 0.0 }
    }
}

impl Component for Position {
    fn id() -> ComponentId {
        ComponentId(1)
    }
    const STORAGE: ComponentStorage = ComponentStorage::SoA;
    type Registry = ();
}

/// 测试 Archetype 定义：固定组件集仅 Position，实体类型 1。
static PLAYER_DEF: ArchetypeDef = ArchetypeDef {
    id: ArchetypeId(0),
    name: "PlayerArchetype",
    component_ids: &[ComponentId(1)],
    entity_kind: EntityTypeId(1),
    component_types: &[],
};

/// 热路径实体数（预分配容量内，避免循环中扩容）。
const ENTITY_COUNT: usize = 1024;
/// 热路径循环次数。
const HOT_ITERATIONS: usize = 1000;

#[test]
fn hot_path_is_zero_allocation() {
    // 串行化：本测试与 `hot_path_query_is_zero_allocation` 共享全局分配计数器。
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let mut world = World::new();
    world.register_archetype(&PLAYER_DEF);

    // 应用级预分配：实体槽位容量一次性到位（R3.4），spawn 阶段零扩容
    world.reserve_entities(ArchetypeId(0), ENTITY_COUNT);

    // 首次 spawn + insert：允许分配（列创建 / HashMap 初始化 / 扩容）
    let mut entities = Vec::with_capacity(ENTITY_COUNT);
    for _ in 0..ENTITY_COUNT {
        entities.push(world.spawn(ArchetypeId(0)));
    }
    for &e in &entities {
        world.insert(e, Position { x: 1.0, y: 1.0 }).unwrap();
    }

    // 预热：确保 HashMap/迭代器等惰性初始化（如容量快照）全部完成
    let _ = world.component_capacity();
    assert_eq!(world.entity_count(), ENTITY_COUNT);

    // ---- 热路径：零分配区间 ----
    ALLOC_COUNT.store(0, Ordering::Relaxed);
    for _ in 0..HOT_ITERATIONS {
        // 每 tick 位置更新：get_mut 原地修改（热路径核心，R14.5）
        for &e in &entities {
            world.get_mut::<Position>(e).unwrap().x += 1.0;
        }
        // 组件读取（等价列迭代）：逐实体 get，O(1) 命中
        for &e in &entities {
            let pos = world.get::<Position>(e).unwrap();
            let _ = pos.x;
        }
        // 列容量迭代（内存统计，R13.2 数据源）
        let _ = world.component_capacity();
    }
    let allocations = ALLOC_COUNT.load(Ordering::Relaxed);
    // ---- 热路径结束 ----

    assert_eq!(
        allocations, 0,
        "热路径发生 {allocations} 次堆分配（期望 0，R3.4 零分配契约被破坏）"
    );

    // ---- query 风格列迭代：零分配 ----
    // T4 Query 遍历的存储层等价物——直接迭代 SoA 列（U1 无界访问路径），
    // 聚合求和不产生中间容器
    let column = SoAColumn::<Position>::with_defaults(ENTITY_COUNT);
    ALLOC_COUNT.store(0, Ordering::Relaxed);
    for _ in 0..HOT_ITERATIONS {
        let sum: f32 = column.iter().map(|p| p.x).sum();
        let _ = sum;
    }
    let column_allocations = ALLOC_COUNT.load(Ordering::Relaxed);
    assert_eq!(
        column_allocations, 0,
        "列迭代发生 {column_allocations} 次堆分配（期望 0，R14.5 无分支迭代被破坏）"
    );
}

/// 全管线零分配：Query 构造一次（允许构造期分配），热循环内跨实体迭代 + 原地
/// 更新（位置 += 速度），断言**分配计数保持不变（0 次）**（T15.1 / R3.4）。
///
/// 这是比 `hot_path_is_zero_allocation` 更贴近运行时的场景：真实系统每 tick
/// 构造一次 Query、迭代全部匹配实体并就地修改组件，全程不分配。
#[test]
fn hot_path_query_is_zero_allocation() {
    // 串行化：本测试与 `hot_path_is_zero_allocation` 共享全局分配计数器。
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let mut world = World::new();
    world.register_archetype(&PLAYER_DEF);
    world.reserve_entities(ArchetypeId(0), ENTITY_COUNT);

    // 预热：spawn + insert（允许分配）
    let mut entities = Vec::with_capacity(ENTITY_COUNT);
    for _ in 0..ENTITY_COUNT {
        entities.push(world.spawn(ArchetypeId(0)));
    }
    for &e in &entities {
        world.insert(e, Position { x: 1.0, y: 1.0 }).unwrap();
    }
    assert_eq!(world.entity_count(), ENTITY_COUNT);

    // 只读 Query 热路径：构造一次（允许构造期分配——扫描 Archetype 并构建匹配
    // 集合），热循环内仅迭代已匹配集合（静态 Archetype 循环，零分配）。预热首次
    // 迭代以排除惰性初始化，随后才进入零分配计数区间。
    {
        let read_query = world.query::<(&Position,), ()>();
        let _ = read_query.iter().count(); // 预热（首次迭代若触发惰性初始化，排除在计数外）
        ALLOC_COUNT.store(0, Ordering::Relaxed);
        for _ in 0..HOT_ITERATIONS {
            let mut sum = 0.0f32;
            for (pos,) in read_query.iter() {
                sum += pos.x;
            }
            let _ = sum;
        }
        let read_alloc = ALLOC_COUNT.load(Ordering::Relaxed);
        assert_eq!(
            read_alloc, 0,
            "只读 Query 热路径发生 {read_alloc} 次堆分配（期望 0，T15.1 全管线零分配被破坏）"
        );
    }

    // 可变 Query 热路径：构造一次（允许分配），iter_mut 仅拆借列引用（零分配）。
    // 预热首次 iter_mut 以排除惰性初始化。
    {
        let mut write_query = world.query_mut::<(&mut Position,), ()>();
        let _ = write_query.iter_mut().count(); // 预热
        ALLOC_COUNT.store(0, Ordering::Relaxed);
        for _ in 0..HOT_ITERATIONS {
            for (pos,) in write_query.iter_mut() {
                pos.x += 1.0;
            }
        }
        let write_alloc = ALLOC_COUNT.load(Ordering::Relaxed);
        assert_eq!(
            write_alloc, 0,
            "可变 Query 热路径发生 {write_alloc} 次堆分配（期望 0，T15.1 全管线零分配被破坏）"
        );
    }

    // 更新应已落盘：每实体 x 增加 HOT_ITERATIONS
    let q = world.query::<(&Position,), ()>();
    let expected = 1.0 + HOT_ITERATIONS as f32;
    for (pos,) in q.iter() {
        assert_eq!(pos.x, expected);
    }
}
