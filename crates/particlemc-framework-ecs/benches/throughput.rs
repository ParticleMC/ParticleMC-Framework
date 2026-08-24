// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 手写基准（T15.3）：spawn 吞吐 / query 迭代 / 单 tick 耗时 / 跨世界迁移成本。
//!
//! 零外部依赖，仅用 `std::time` 计时；运行：`cargo bench --bench throughput`。
//! 各基准打印 `per_op` 纳秒，供 `docs/benchmarks.md` 记录与调优对比。
//!
//! 变更标识符：`implement-custom-ecs`

use std::hint::black_box;
use std::time::Instant;

use particlemc_framework_ecs::archetype::{ArchetypeDef, ArchetypeId};
use particlemc_framework_ecs::component::{Component, ComponentId, ComponentStorage};
use particlemc_framework_ecs::entity::{Entity, EntityTypeId};
use particlemc_framework_ecs::migration::migrate_entity;
use particlemc_framework_ecs::query::Query;
use particlemc_framework_ecs::schedule::Schedule;
use particlemc_framework_ecs::world::World;

/// 基准实体规模。
const N: usize = 10_000;

// ---- 基准组件（手工实现，避免依赖宏 crate）----

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

static PLAYER_DEF: ArchetypeDef = ArchetypeDef {
    id: ArchetypeId(0),
    name: "BenchPlayer",
    component_ids: &[ComponentId(1), ComponentId(2)],
    entity_kind: EntityTypeId(1),
    component_types: &[],
};

/// 构造含 `n` 个（Position + Velocity）实体的世界。
fn build_world(n: usize) -> World {
    let mut world = World::new();
    world.register_archetype(&PLAYER_DEF);
    world.reserve_entities(ArchetypeId(0), n);
    for _ in 0..n {
        let e = world.spawn(ArchetypeId(0));
        let _ = world.insert(e, Position { x: 1.0, y: 1.0 });
        let _ = world.insert(e, Velocity { dx: 0.1, dy: 0.0 });
    }
    world
}

/// 计时辅助：预热一次后测量 `iters` 次平均 `per_op` 纳秒。
/// 接受 `FnMut`：基准闭包常 mutate 捕获的世界/列/调度器（如 tick、SIMD 加法）。
fn time_it(label: &str, iters: u32, mut f: impl FnMut()) {
    f(); // 预热（含首次惰性建列 / 编译期分支稳定）
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    let elapsed = start.elapsed();
    let per_op = elapsed.as_nanos() as f64 / iters as f64;
    println!(
        "[{label}] iters={iters} total={:?} per_op={:.1} ns",
        elapsed, per_op
    );
}

/// spawn 吞吐：N 实体 spawn + 双组件 insert（含惰性建列）。
fn bench_spawn_throughput() {
    let iters = 50u32;
    time_it("spawn_throughput", iters, || {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        world.reserve_entities(ArchetypeId(0), N);
        for _ in 0..N {
            let e = world.spawn(ArchetypeId(0));
            let _ = world.insert(e, Position { x: 1.0, y: 1.0 });
            let _ = world.insert(e, Velocity { dx: 0.1, dy: 0.0 });
        }
        black_box(&world);
    });
}

/// query 迭代：构造一次 Query、遍历 N 实体只读求和。
fn bench_query_iter() {
    let world = build_world(N);
    let iters = 2000u32;
    time_it("query_iter", iters, || {
        let q: Query<(&Position,), ()> = world.query();
        let mut sum = 0.0f32;
        for (p,) in q.iter() {
            sum += p.x;
        }
        black_box(sum);
    });
}

/// 单 tick 耗时：含位置/速度更新的物理系统经 Schedule 执行一轮。
fn bench_single_tick() {
    let mut world = build_world(N);
    let mut schedule = Schedule::new();
    schedule.add_system(|mut q: Query<(&mut Position, &Velocity)>| {
        for (pos, vel) in q.iter_mut() {
            pos.x += vel.dx;
        }
    });
    let iters = 2000u32;
    time_it("single_tick", iters, || {
        schedule.run(&mut world);
    });
}

/// 跨世界迁移成本（IC-12）：将 N 实体从源世界迁移到目标世界（组件全量随迁）。
fn bench_migration_cost() {
    // 仅取实体 id 列表作为迁移目标集合（实体句柄跨世界稳定，逻辑标识不变）
    let entities: Vec<Entity> = {
        let w = build_world(N);
        let q: Query<(Entity, &Position), ()> = w.query();
        q.iter().map(|(e, _)| e).collect()
    };

    let iters = 200u32;
    time_it("migration_cost", iters, || {
        // 每次重建源世界以模拟"迁移 N 实体"的完整成本（含 despawn + 重 spawn）
        let mut s = build_world(N);
        let mut d = World::new();
        d.register_archetype(&PLAYER_DEF);
        let p = d.spawn(ArchetypeId(0));
        let _ = d.insert(p, Position::default());
        let _ = d.insert(p, Velocity::default());
        for &e in &entities {
            let _ = migrate_entity(&mut s, &mut d, e);
        }
        black_box(&d);
    });
}

fn main() {
    println!("=== 吞吐 / 迭代 / tick / 迁移基准（N={N}） ===");
    bench_spawn_throughput();
    bench_query_iter();
    bench_single_tick();
    bench_migration_cost();
    println!("=== 结束 ===");
}
