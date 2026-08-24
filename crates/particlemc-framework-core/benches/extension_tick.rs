// 扩展 tick 派发性能基准（WS4-T1 G5，ADR-016 §5 tick 性能预算）。
//
// 本基准测量单个 tick 派发 N 个扩展回调的成本（tens~hundreds ns 级别）。
// 20Hz（50ms）预算下，64 个扩展累积开销应可忽略。

#![cfg_attr(not(feature = "wasm-extensions"), allow(dead_code, unused_variables))]

#[cfg(feature = "wasm-extensions")]
use criterion::{Criterion, black_box, criterion_group, criterion_main};
#[cfg(feature = "wasm-extensions")]
use particlemc_framework_core::extension::ExtensionManager;

#[cfg(feature = "wasm-extensions")]
fn bench_tick_dispatch(c: &mut Criterion) {
    // 空管理器（无扩展）测量开销基准。
    let mut group = c.benchmark_group("extension_tick");
    group.bench_function("empty_manager", |b| {
        let manager = ExtensionManager::new();
        b.iter(|| manager.tick_all());
    });

    // TODO: 后续可构造真实 `.wasm` 扩展（通过 `wat` dev-dep）测量带扩展的派发成本。
    // v1 仅测空管理器开销，验证 tick_all 本身不引入额外负担。
}

#[cfg(feature = "wasm-extensions")]
criterion_group!(benches, bench_tick_dispatch);
#[cfg(feature = "wasm-extensions")]
criterion_main!(benches);

#[cfg(not(feature = "wasm-extensions"))]
fn main() {
    println!("feature `wasm-extensions` 未启用，跳过基准测试");
}
