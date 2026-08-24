<!-- Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
SPDX-License-Identifier: GPL-3.0-or-later -->
# Spec 副本：load-generic-and-loot-registries

> 权威记录。原 `.specs/load-generic-and-loot-registries/` 目录两次被外部机制删除（项目非 git 仓库），此副本防止再次丢失。

---

# 补齐 generic/ 与 loot_tables/ 注册数据加载 Specification

## AI Amendment Log
- 2026-08-08（Amendment 1）：`generic/` 实际 **44 文件、969 entry、合并唯一键 923**（46 处跨文件同名键被后读覆盖）。R1/R4 中 "969" 修正为 "923（唯一键）"。

## Why
`resources/data/` 共 76 个注册数据文件，当前仅 23 个被加载（顶层 11 个注册表 + `tags/` 12 个）。
`generic/`（44 文件、969 entry、唯一键 923）与 `loot_tables/`（4 文件、1275 entry）从未进入资源系统。
补齐加载使 1.21.11 全量注册数据可用。

## What Changes
- generic.rs：新增 `load_directory`；无 name 条目回退 id 十进制字符串键
- 新增 loot.rs：`LootTable` 轻量类型 + `LootTableRegistry`
- 两个 mod.rs：导出 LootTable / LootTableRegistry
- plugin.rs：new() 与 with_preload() 都加载 generic/ 与 loot_tables/（始终加载）
- registry_data_integration.rs：断言实际条目数
- 文档：README / CHANGELOG / ADR-008

## Impact
- 直接：generic.rs、loot.rs（新）、registries/mod.rs、resource/mod.rs、plugin.rs、registry_data_integration.rs
- 间接：README.md / CHANGELOG.md / docs/decisions.md

## ADDED Requirements

### Requirement: GenericRegistry 目录加载
系统 SHALL 提供 `GenericRegistry::load_directory(dir: &Path) -> Result<Self, RegistryError>`，遍历目录所有 `.toml` 合并条目（同键后者覆盖），语义对齐 `TagRegistry::load_directory`。
#### Scenario: 加载 generic 目录全部文件
- **WHEN** 调用 `load_directory("resources/data/generic")`
- **THEN** 返回 44 个文件合并结果，唯一键总数 = 923（含 sound_sources 的 11 条 id 键）
#### Scenario: 无 name 条目以 id 兜底
- **WHEN** 条目无 `name` 但有 `id`
- **THEN** 以 `id` 十进制字符串（如 `"0"`）为键入库
#### Scenario: 目录不可读
- **WHEN** `dir` 不存在或不可读
- **THEN** 返回 `RegistryError::ParseError`（调用方 unwrap_or_default 回退，不 panic）

### Requirement: LootTableRegistry 类型与解析
系统 SHALL 提供轻量 `LootTable { name, table_type, random_sequence, pools: Option<toml::Value>, raw: toml::Value }` 与 `LootTableRegistry`（entries: HashMap，含 from_toml_str/from_toml_file/load_directory/get/len/is_empty）。
#### Scenario: 加载 loot_tables 目录全部文件
- **WHEN** 调用 `load_directory("resources/data/loot_tables")`
- **THEN** 返回 4 文件合并的 1275 条 loot table，按 name 索引
#### Scenario: entry 顶层附加字段不丢失
- **WHEN** entry 有顶层附加字段（如 block 表 9 条 entry 级 functions）
- **THEN** `raw` 保留完整原值，`pools` 可经 toml::Value 读取
#### Scenario: 解析失败
- **WHEN** 文本非法或缺 name
- **THEN** 返回 `RegistryError::ParseError`（缺 name 跳过该条；调用方回退不 panic）

### Requirement: 插件始终加载 generic 与 loot
系统 SHALL 在 `McServerPlugin::new()` 与 `with_preload()` 都装配非空 GenericRegistry 与 LootTableRegistry。
#### Scenario: new() 装配全量数据
- **WHEN** 以 new() 构建 App
- **THEN** GenericRegistry 唯一键 = 923，LootTableRegistry = 1275
#### Scenario: with_preload() 装配全量数据
- **WHEN** 以 with_preload() 构建 App
- **THEN** 同样 923 / 1275，既有 8 个世界类注册表不受影响
#### Scenario: 数据目录缺失
- **WHEN** data_dir/generic 或 loot_tables 不存在
- **THEN** 对应注册表回退空表，App 正常构建不 panic

## Interface Contracts

### Interface: GenericRegistry::load_directory
- Module: crates/minestom-core/src/resource/registries/generic.rs
- Signature: `pub fn load_directory(dir: &Path) -> Result<Self, RegistryError>`
- 语义：目录不可读/文件解析失败 → ParseError；无 name 回退 id；同键后者覆盖
- Owner: T1; Consumer: T3

### Interface: LootTable / LootTableRegistry
- Module: crates/minestom-core/src/resource/registries/loot.rs
- 结构：`LootTable { name, table_type, random_sequence, pools: Option<toml::Value>, raw: toml::Value }`；`LootTableRegistry { entries: HashMap<String, LootTable> }`
- 方法：from_toml_str / from_toml_file / load_directory / get / len / is_empty
- 语义：缺 name 跳过；raw 恒整条 entry；解析失败 ParseError
- Owner: T2; Consumer: T3

## Compatibility & Rollback
- 外部接口变更：无（仅新增 API）；兼容性：new() 额外加载两表，失败回退空表不影响既有行为
- 回滚：还原变更文件即可；验证 cargo test --workspace 全绿 + clippy 门禁
