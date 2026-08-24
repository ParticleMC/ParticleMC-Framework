// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 手写基准（T16.1 / T16.2 / T16.5）：SIMD vs 标量、静态 archetype 匹配 vs
//! 运行时逐实体查找、列 iter 无分支 vs 逐元素 get 边界检查。
//!
//! 零外部依赖，仅用 `std::time` 计时；运行：`cargo bench --bench optimizations`。
//! 各基准打印 `per_op` 纳秒，供 `docs/benchmarks.md` 记录与调优对比。
//!
//! 变更标识符：`implement-custom-ecs`

use std::hint::black_box;
use std::time::Instant;

use particlemc_framework_ecs::archetype::{ArchetypeDef, ArchetypeId};
use particlemc_framework_ecs::component::{Component, ComponentId, ComponentStorage};
use particlemc_framework_ecs::entity::{Entity, EntityTypeId};
use particlemc_framework_ecs::query::Query;
use particlemc_framework_ecs::storage::soa::SoAColumn;
use particlemc_framework_ecs::world::World;

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
    name: "BenchPlayerOpt",
    component_ids: &[ComponentId(1), ComponentId(2)],
    entity_kind: EntityTypeId(1),
    component_types: &[],
};

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

fn time_it(label: &str, iters: u32, mut f: impl FnMut()) {
    f();
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

/// T16.2：SIMD（AVX2 8 路）批量加法 vs 标量回退，对比每元素加法成本。
fn bench_simd_vs_scalar() {
    const M: usize = 10_000;
    let rhs: Vec<f32> = (0..M).map(|i| (i % 7) as f32).collect();
    let mut scalar_col = SoAColumn::<f32>::with_defaults(M);
    for i in 0..M {
        scalar_col.set(i, i as f32);
    }
    let mut simd_col = SoAColumn::<f32>::with_defaults(M);
    for i in 0..M {
        simd_col.set(i, i as f32);
    }
    let iters = 5000u32;
    time_it("simd_scalar_add", iters, || {
        scalar_col.add_assign_scalar(&rhs);
        black_box(&scalar_col);
    });
    #[cfg(target_arch = "x86_64")]
    {
        time_it("simd_avx2_add", iters, || {
            // SAFETY: rhs.len() == M == self.len()，满足 add_assign_simd 前置
            unsafe { simd_col.add_assign_simd(&rhs) };
            black_box(&simd_col);
        });
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        println!("[simd_avx2_add] 跳过：非 x86_64 平台，使用标量回退（T16.2 跨平台回退验证）");
    }
}

/// T16.1：静态 archetype 匹配（Query 构造一次、迭代预计算匹配集）vs 运行时
/// 逐实体 `World::get`（每次重新做 entity_index + 列查找）。
fn bench_static_vs_runtime_query() {
    let world = build_world(N);
    let entities: Vec<Entity> = {
        let q: Query<(Entity, &Position), ()> = world.query();
        q.iter().map(|(e, _)| e).collect()
    };
    let iters = 2000u32;
    time_it("static_query_iter", iters, || {
        let q: Query<(&Position,), ()> = world.query();
        let mut sum = 0.0f32;
        for (p,) in q.iter() {
            sum += p.x;
        }
        black_box(sum);
    });
    time_it("runtime_get_per_entity", iters, || {
        let mut sum = 0.0f32;
        for &e in &entities {
            if let Some(p) = world.get::<Position>(e) {
                sum += p.x;
            }
        }
        black_box(sum);
    });
}

/// T16.5：列 `iter`（无分支、U1 无界访问）vs 逐元素 `get`（每次边界检查）。
fn bench_branchless_vs_branching() {
    const M: usize = 10_000;
    let col = SoAColumn::<f32>::with_defaults(M);
    let iters = 5000u32;
    time_it("column_iter_branchless", iters, || {
        let mut sum = 0.0f32;
        for v in col.iter() {
            sum += *v;
        }
        black_box(sum);
    });
    time_it("column_get_branching", iters, || {
        let mut sum = 0.0f32;
        for i in 0..M {
            if let Some(v) = col.get(i) {
                sum += *v;
            }
        }
        black_box(sum);
    });
}

fn main() {
    println!("=== 优化基准（SIMD / 静态展开 / 无分支） ===");
    bench_simd_vs_scalar();
    bench_static_vs_runtime_query();
    bench_branchless_vs_branching();
    println!("=== 结束 ===");
}
