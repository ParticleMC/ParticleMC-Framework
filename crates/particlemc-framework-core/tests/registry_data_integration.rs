// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 集成测试：完整注册数据加载计数校验。
//!
//! 验证从 `resources/data/` 加载的 JSON 注册数据可被正确解析，
//! 仅做计数校验（不逐条断言），避免测试对大体量数据产生脆弱依赖。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use particlemc_framework_core::app::App;
use particlemc_framework_core::plugin::McServerPlugin;
use particlemc_framework_core::resource::registries::{BlockRegistry, EntityTypeRegistry, ItemRegistry};
use particlemc_framework_core::resource::{GenericRegistry, LootTableRegistry};

/// 返回 ParticleMC 数据目录（相对当前 crate 的 `CARGO_MANIFEST_DIR`）。
fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/data")
}

#[test]
fn blocks_data_loads_with_full_count() {
    let registry = BlockRegistry::from_json_file(&data_dir().join("blocks.json"))
        .expect("blocks.json 应可被加载");
    assert!(
        registry.0.len() >= 1000,
        "方块数应 ≥ 1000，实际为 {}",
        registry.0.len()
    );
}

#[test]
fn items_data_loads_with_full_count() {
    let registry = ItemRegistry::from_json_file(&data_dir().join("items.json"))
        .expect("items.json 应可被加载");
    assert!(
        registry.0.len() >= 1000,
        "物品数应 ≥ 1000，实际为 {}",
        registry.0.len()
    );
}

#[test]
fn entity_types_data_loads_with_full_count() {
    let registry = EntityTypeRegistry::from_json_file(&data_dir().join("entity_types.json"))
        .expect("entity_types.json 应可被加载");
    assert!(
        registry.0.len() >= 100,
        "实体类型数应 ≥ 100，实际为 {}",
        registry.0.len()
    );
}

/// 校验默认构造路径装配的 generic/ 与 loot_tables/ 注册数据条数。
///
/// 注：generic/ 目录含 969 条数据，但存在跨文件同名键，按
/// `load_json_directory` 的合并语义（后者覆盖前者）去重后实际为 926 条；此处以
/// 实际加载数为准（数据文件是权威）。
#[test]
fn plugin_new_loads_generic_and_loot_tables_with_full_count() {
    let mut app = App::new();
    app.add_plugins(McServerPlugin::new());

    let generic = app
        .world()
        .get_resource::<GenericRegistry>()
        .expect("GenericRegistry 应已装配");
    assert_eq!(
        generic.len(),
        926,
        "generic/ 条目数应为 926，实际为 {}",
        generic.len()
    );

    let loot = app
        .world()
        .get_resource::<LootTableRegistry>()
        .expect("LootTableRegistry 应已装配");
    assert_eq!(
        loot.len(),
        1273,
        "loot_tables/ 条目数应为 1273，实际为 {}",
        loot.len()
    );
}

/// 校验全量预热构造路径同样装配 generic/ 与 loot_tables/ 注册数据条数。
#[test]
fn plugin_with_preload_loads_generic_and_loot_tables_with_full_count() {
    let mut app = App::new();
    app.add_plugins(McServerPlugin::with_preload());

    let generic = app
        .world()
        .get_resource::<GenericRegistry>()
        .expect("GenericRegistry 应已装配");
    assert_eq!(
        generic.len(),
        926,
        "generic/ 条目数应为 926，实际为 {}",
        generic.len()
    );

    let loot = app
        .world()
        .get_resource::<LootTableRegistry>()
        .expect("LootTableRegistry 应已装配");
    assert_eq!(
        loot.len(),
        1273,
        "loot_tables/ 条目数应为 1273，实际为 {}",
        loot.len()
    );
}
