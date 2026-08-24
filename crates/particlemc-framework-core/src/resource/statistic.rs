// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 统计体系（T13，对应 spec R13）。
//!
//! [`Statistic`] 为数据驱动注册条目（`resources/data/statistics.toml`），
//! `category` 对齐 Java `StatisticCategory` 枚举序位（0=MINED、1=CRAFTED、
//! 2=USED、3=BROKEN、4=PICKED_UP、5=DROPPED、6=KILLED、7=KILLED_BY、
//! 8=CUSTOM），`id` 为对应子注册表序位（方块 / 物品 / 实体类型 /
//! `custom_statistics`）。[`StatisticRegistry`] 仿 `DamageTypeRegistry`
//! 模式加载 TOML，提供 `by_name` / `by_key` 查询。
//!
//! [`PlayerStatistics`] 为挂载在玩家实体上的组件，以统计 id 为键记录数值，
//! 以 `dirty` 集合标记自上次下发以来发生变更的统计，经
//! [`to_packet`](PlayerStatistics::to_packet) 生成 `Statistics`(0x03) 包。
//!
//! 变更标识符：`complete-missing-subsystems`（T13）。

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::prelude::Component;
use serde::Deserialize;

use crate::protocol::packets::play::Statistics;
use crate::resource::registries::RegistryError;

/// 单条统计定义（注册表条目）。
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Statistic {
    /// 命名空间名称，如 `minecraft:walk_one_cm`。
    pub name: String,
    /// 统计 id（子注册表序位，语义见模块文档）。
    pub id: u32,
    /// 统计类别（Java `StatisticCategory` 枚举序位 0..=8）。
    pub category: u32,
}

/// 统计注册表：name → [`Statistic`]。
///
/// 统计 id 在不同类别下可复用（如 CUSTOM 的 `play_time`=1 与 MINED 的
/// `mine_block`=1），故不采用全局「id → 值」映射，而以 name 为主键。
#[derive(Debug, Clone, Default)]
pub struct StatisticRegistry {
    /// name → 统计定义。
    by_name: HashMap<String, Statistic>,
}

impl StatisticRegistry {
    /// 构造一个空注册表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 从 TOML 文本解析 `[[entry]]` 数组，构建统计注册表。
    ///
    /// # 错误
    /// 文本非法、结构不符或 name 重复时返回 [`RegistryError`]。
    pub fn from_toml_str(text: &str) -> Result<Self, RegistryError> {
        let document: toml::Value = toml::from_str(text).map_err(|_| RegistryError::ParseError)?;
        let entries = document
            .get("entry")
            .and_then(|value| value.as_array())
            .ok_or(RegistryError::ParseError)?;
        let mut by_name = HashMap::with_capacity(entries.len());
        for entry in entries {
            let stat: Statistic = entry
                .clone()
                .try_into()
                .map_err(|_: toml::de::Error| RegistryError::ParseError)?;
            if by_name.contains_key(&stat.name) {
                return Err(RegistryError::DuplicateName);
            }
            by_name.insert(stat.name.clone(), stat);
        }
        Ok(Self { by_name })
    }

    /// 从单个 TOML 文件加载统计注册表。
    ///
    /// # 错误
    /// 文件缺失或解析失败返回 [`RegistryError::ParseError`]。
    pub fn from_toml_file(path: &Path) -> Result<Self, RegistryError> {
        let text = std::fs::read_to_string(path).map_err(|_| RegistryError::ParseError)?;
        Self::from_toml_str(&text)
    }

    /// 按命名空间名称查询统计定义。
    pub fn by_name(&self, name: &str) -> Option<&Statistic> {
        self.by_name.get(name)
    }

    /// 按 `(category, id)` 组合反查统计定义（线格式包条目语义）。
    pub fn by_key(&self, category: u32, id: u32) -> Option<&Statistic> {
        self.by_name
            .values()
            .find(|stat| stat.category == category && stat.id == id)
    }

    /// 已注册统计条目数量。
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    /// 注册表是否为空。
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

/// 玩家统计值容器（挂载在玩家实体上的组件）。
///
/// 以统计 id 为键记录数值；`dirty` 记录自上次下发以来变更过的统计 id，
/// 供统计同步按需生成增量 `Statistics` 包。
#[derive(Debug, Clone, Default, Component)]
#[component(storage = "sparse")]
pub struct PlayerStatistics {
    /// 统计 id → 数值。
    pub values: HashMap<u32, u32>,
    /// 自上次下发以来变更过的统计 id。
    pub dirty: HashSet<u32>,
}

impl PlayerStatistics {
    /// 构造一个空的统计容器。
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置统计值并标记脏。
    pub fn set_statistic(&mut self, id: u32, value: u32) {
        self.values.insert(id, value);
        self.dirty.insert(id);
    }

    /// 查询统计值（未设置时返回 0）。
    pub fn get_statistic(&self, id: u32) -> u32 {
        self.values.get(&id).copied().unwrap_or(0)
    }

    /// 对统计值做饱和累加并标记脏。
    pub fn increment(&mut self, id: u32, by: u32) {
        let next = self.get_statistic(id).saturating_add(by);
        self.set_statistic(id, next);
    }

    /// 指定统计自上次下发以来是否发生变更。
    pub fn is_dirty(&self, id: u32) -> bool {
        self.dirty.contains(&id)
    }

    /// 当前脏统计 id 的迭代器（顺序不定）。
    pub fn dirty_ids(&self) -> impl Iterator<Item = &u32> {
        self.dirty.iter()
    }

    /// 清空脏标记（下发完成后调用）。
    pub fn clear_dirty(&mut self) {
        self.dirty.clear();
    }

    /// 将所有脏统计编码为 `Statistics`(0x03) 包。
    ///
    /// 每条目为 `(category, statisticId, value)`。类别经注册表反查：按
    /// `(category, id)` 匹配；未注册的统计回退到 CUSTOM(8) 类别。
    pub fn to_packet(&self, registry: &StatisticRegistry) -> Statistics {
        let mut entries = Vec::with_capacity(self.dirty.len());
        for &id in &self.dirty {
            let Some(&value) = self.values.get(&id) else {
                continue;
            };
            let category = registry
                .by_name
                .values()
                .find(|stat| stat.id == id)
                .map(|stat| stat.category)
                .unwrap_or(8);
            entries.push((
                i32::try_from(category).unwrap_or(i32::MAX),
                i32::try_from(id).unwrap_or(i32::MAX),
                i32::try_from(value).unwrap_or(i32::MAX),
            ));
        }
        Statistics { entries }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATISTICS_SMALL: &str = r#"
[[entry]]
name = "minecraft:walk_one_cm"
category = 8
id = 6

[[entry]]
name = "minecraft:mine_block"
category = 0
id = 1

[[entry]]
name = "minecraft:kill_entity"
category = 6
id = 150
"#;

    #[test]
    fn from_toml_str_loads_entries() {
        let registry = StatisticRegistry::from_toml_str(STATISTICS_SMALL).unwrap();
        assert_eq!(registry.len(), 3);
        let walk = registry.by_name("minecraft:walk_one_cm").unwrap();
        assert_eq!(walk.id, 6);
        assert_eq!(walk.category, 8);
        assert_eq!(
            registry.by_key(6, 150).map(|stat| stat.name.as_str()),
            Some("minecraft:kill_entity")
        );
        assert!(registry.by_name("minecraft:missing").is_none());
        assert!(registry.by_key(0, 999).is_none());
    }

    #[test]
    fn from_toml_str_rejects_duplicate_name() {
        let text = r#"
[[entry]]
name = "minecraft:jump"
category = 8
id = 23

[[entry]]
name = "minecraft:jump"
category = 8
id = 24
"#;
        assert!(matches!(
            StatisticRegistry::from_toml_str(text),
            Err(RegistryError::DuplicateName)
        ));
    }

    #[test]
    fn from_toml_str_rejects_malformed() {
        assert!(matches!(
            StatisticRegistry::from_toml_str("this is not = = valid toml @@@"),
            Err(RegistryError::ParseError)
        ));
        assert!(matches!(
            StatisticRegistry::from_toml_str("name = \"oops\""),
            Err(RegistryError::ParseError)
        ));
    }

    #[test]
    fn from_toml_file_missing_path_returns_parse_error() {
        let missing = Path::new("does/not/exist/statistics.toml");
        assert!(matches!(
            StatisticRegistry::from_toml_file(missing),
            Err(RegistryError::ParseError)
        ));
    }

    #[test]
    fn real_data_file_loads() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/data/statistics.toml");
        let registry = StatisticRegistry::from_toml_file(&path).unwrap();
        assert!(registry.len() >= 5, "实际 {} 条", registry.len());
        assert_eq!(registry.by_name("minecraft:walk_one_cm").unwrap().id, 6);
        assert_eq!(
            registry.by_name("minecraft:mine_block").unwrap().category,
            0
        );
    }

    #[test]
    fn player_statistics_set_get_increment_dirty() {
        let mut stats = PlayerStatistics::new();
        assert_eq!(stats.get_statistic(6), 0);
        assert!(!stats.is_dirty(6));

        stats.set_statistic(6, 100);
        assert_eq!(stats.get_statistic(6), 100);
        assert!(stats.is_dirty(6));

        stats.increment(6, 25);
        assert_eq!(stats.get_statistic(6), 125);

        // 饱和累加不溢出。
        stats.set_statistic(33, u32::MAX);
        stats.increment(33, 10);
        assert_eq!(stats.get_statistic(33), u32::MAX);

        stats.clear_dirty();
        assert!(!stats.is_dirty(6));
        assert!(!stats.is_dirty(33));
    }

    #[test]
    fn to_packet_encodes_dirty_entries_with_categories() {
        let registry = StatisticRegistry::from_toml_str(STATISTICS_SMALL).unwrap();
        let mut stats = PlayerStatistics::new();
        stats.set_statistic(6, 500); // walk_one_cm → CUSTOM(8)
        stats.increment(1, 2); // mine_block(stone) → MINED(0)
        stats.set_statistic(9999, 7); // 未注册 → 回退 CUSTOM(8)

        let packet = stats.to_packet(&registry);
        let mut entries = packet.entries.clone();
        entries.sort();
        assert_eq!(entries.len(), 3);
        assert!(entries.contains(&(8, 6, 500)));
        assert!(entries.contains(&(0, 1, 2)));
        assert!(entries.contains(&(8, 9999, 7)));

        // 下发后清脏：再次 to_packet 应为空。
        stats.clear_dirty();
        assert!(stats.to_packet(&registry).entries.is_empty());
    }
}
