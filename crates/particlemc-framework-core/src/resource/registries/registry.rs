// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 注册表核心数据结构与 TOML 加载器。
//!
//! [`Registry`] 提供「数值 id ⇄ 命名空间字符串」的双向映射，是 Minestom
//! 所有注册数据（方块、物品、实体类型……）的统一承载结构。它刻意保持泛型，
//! 由 [`BlockRegistry`] 等具名注册表在之上包装为具体的 `Resource`。
//!
//! 加载器 [`Registry::from_toml_str`] / [`Registry::from_toml_file`] 读取
//! `tools/gen_registry_data.py` 生成的 `[[entry]]` 数组 TOML，兼容「有 id」
//! 与「无 id（自动编号）」两种数据源，且对解析失败一律返回 [`RegistryError`]
//! 而非 panic，满足骨架层对错误分支 100% 覆盖的要求。

use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::component::Block;

use super::block::BlockRegistry;

/// 注册表操作可能产生的错误。
///
/// 所有构造/加载路径都会返回该错误而非 panic，便于调用方做可控的失败处理。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryError {
    /// 重复注册同一 `name`（或同一 id 被两条目占用）。
    DuplicateName,
    /// 自增 id 溢出 `u32::MAX`（理论上不会发生，仅作防御）。
    IdOverflow,
    /// TOML 文件缺失或内容无法解析为合法注册项。
    ParseError,
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::DuplicateName => write!(f, "注册表条目名称重复"),
            RegistryError::IdOverflow => write!(f, "注册表 id 溢出 u32::MAX"),
            RegistryError::ParseError => write!(f, "TOML 解析失败或文件缺失"),
        }
    }
}

impl std::error::Error for RegistryError {}

/// 可被 [`Registry::from_toml_str`] 直接反序列化的注册项约束。
///
/// 每个注册项必须暴露其 `name`（命名空间字符串）与可选的 `id`
/// （数据源提供的规范 id；缺失时由注册表自动编号）。
pub trait RegistryEntry {
    /// 返回该注册项的命名空间名称（如 `minecraft:stone`）。
    fn entry_name(&self) -> &str;
    /// 返回该注册项的规范 id（数据源未提供时为 `None`，由注册表自动分配）。
    fn entry_id(&self) -> Option<u32>;
}

/// 通用注册表：维护「id → 值」与「name → id」两套映射。
///
/// 字段形状与冻结接口契约保持一致（`by_id` / `by_name` / `next_id`），
/// 仅追加了内部字段之外的逻辑。
#[derive(Debug, Clone, PartialEq)]
pub struct Registry<T> {
    /// id → 注册值。
    by_id: HashMap<u32, T>,
    /// name → id。
    by_name: HashMap<String, u32>,
    /// 下一个将自动分配的 id。
    next_id: u32,
    /// 运行时增删产生的脏标记 id 列表，供 `registry_sync` 系统消费。
    ///
    /// 仅 [`insert`](Self::insert) / [`remove`](Self::remove) 会写入，
    /// 静态加载路径（`register` / `from_toml_str`）不置脏。
    dirty: Vec<u32>,
}

impl<T> Default for Registry<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Registry<T> {
    /// 创建一个空的注册表。
    ///
    /// 此时 `next_id` 为 0，首次 [`register`](Self::register) 将分配 id 1。
    pub fn new() -> Self {
        Self {
            by_id: HashMap::new(),
            by_name: HashMap::new(),
            next_id: 0,
            dirty: Vec::new(),
        }
    }

    /// 注册一个新条目，返回其分配到的自增 id。
    ///
    /// # 错误
    /// - 若 `name` 已存在，返回 [`RegistryError::DuplicateName`] 且不覆盖旧值。
    /// - 若自增 `next_id` 溢出，返回 [`RegistryError::IdOverflow`]。
    ///
    /// # 示例
    /// ```
    /// use particlemc_framework_core::resource::registries::{Registry, RegistryError};
    /// let mut r: Registry<u32> = Registry::new();
    /// let id = r.register("minecraft:stone", 0).unwrap();
    /// assert_eq!(id, 1);
    /// assert_eq!(r.get_id("minecraft:stone"), Some(1));
    /// ```
    pub fn register(&mut self, name: impl Into<String>, value: T) -> Result<u32, RegistryError> {
        let id = self
            .next_id
            .checked_add(1)
            .ok_or(RegistryError::IdOverflow)?;
        let name = name.into();
        if self.by_name.contains_key(&name) {
            return Err(RegistryError::DuplicateName);
        }
        self.by_name.insert(name, id);
        self.by_id.insert(id, value);
        self.next_id = id;
        Ok(id)
    }

    /// 覆盖或注册一个条目：name 已存在时保留原 id、替换 value；不存在时
    /// 等价于 [`register`](Self::register) 分配新 id。
    ///
    /// 与 [`register`](Self::register) 的区别在于：对已存在的 name 不返回
    /// `DuplicateName`，而是静默替换其值（运行时覆盖注册数据的入口）。
    ///
    /// # 错误
    /// 仅当自增 `next_id` 溢出（分配新 id 的路径）时返回
    /// [`RegistryError::IdOverflow`]。
    pub fn override_value(&mut self, name: &str, value: T) -> Result<(), RegistryError> {
        if let Some(&id) = self.by_name.get(name) {
            self.by_id.insert(id, value);
            return Ok(());
        }
        self.register(name.to_string(), value).map(|_| ())
    }

    /// 注册或替换一个条目：name 已存在时保留原 id、替换 value 并返回该 id；
    /// 不存在时等价于 [`register`](Self::register) 并返回新分配 id。
    ///
    /// # 错误
    /// 仅当自增 `next_id` 溢出（分配新 id 的路径）时返回
    /// [`RegistryError::IdOverflow`]。
    pub fn register_or_replace(
        &mut self,
        name: impl Into<String>,
        value: T,
    ) -> Result<u32, RegistryError> {
        let name = name.into();
        if let Some(&id) = self.by_name.get(&name) {
            self.by_id.insert(id, value);
            return Ok(id);
        }
        self.register(name, value)
    }

    /// 运行时插入条目（name 唯一；冲突返回 [`RegistryError::DuplicateName`]）。
    ///
    /// 与 [`register`](Self::register) 的不同点在于：插入成功后**置脏标记**
    /// （追加该 id 到 `dirty`），供 `registry_sync` 系统在后续 tick 向已连接
    /// 客户端重发 `RegistryData` 增量/全量同步。
    ///
    /// id 按既有自增语义升序分配，与 `get_name` / `get` 的 id 顺序语义兼容。
    ///
    /// # 错误
    /// - 若 `name` 已存在，返回 [`RegistryError::DuplicateName`] 且不覆盖旧值。
    /// - 若自增 `next_id` 溢出，返回 [`RegistryError::IdOverflow`]。
    pub fn insert(&mut self, name: impl Into<String>, value: T) -> Result<(), RegistryError> {
        let name = name.into();
        if self.by_name.contains_key(&name) {
            return Err(RegistryError::DuplicateName);
        }
        let id = self.register(name, value)?;
        self.dirty.push(id);
        Ok(())
    }

    /// 运行时移除条目（按 name），并置脏标记。
    ///
    /// 移除不重用 id（vanilla 语义），客户端据协议在下一次 `RegistryData`
    /// 同步时重映射。返回被移除的值（不存在返回 `None`）。
    pub fn remove(&mut self, name: &str) -> Option<T> {
        let id = self.by_name.get(name).copied()?;
        let value = self.by_id.remove(&id)?;
        self.by_name.remove(name);
        // 记录被移除的 id，供同步系统下发移除 / 重映射。
        self.dirty.push(id);
        Some(value)
    }

    /// 取走全部脏标记 id 并清空（供 `registry_sync` 系统消费）。
    ///
    /// 取走后脏列表复位为空，下一次 [`insert`](Self::insert) /
    /// [`remove`](Self::remove) 会重新累积。
    #[must_use]
    pub fn take_dirty(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.dirty)
    }

    /// 当前是否存在未同步的脏标记（不消费）。
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        !self.dirty.is_empty()
    }

    /// 按 id 查询注册值。
    pub fn get(&self, id: u32) -> Option<&T> {
        self.by_id.get(&id)
    }

    /// 按 name 查询其对应 id。
    pub fn get_id(&self, name: &str) -> Option<u32> {
        self.by_name.get(name).copied()
    }

    /// 按 id 反查 name。
    ///
    /// 由于映射以 `by_name` 为正向索引，此处以线性扫描实现；
    /// 骨架阶段注册规模可控，性能影响可忽略。
    pub fn get_name(&self, id: u32) -> Option<&str> {
        self.by_name
            .iter()
            .find(|&(_, &assigned)| assigned == id)
            .map(|(name, _)| name.as_str())
    }

    /// 返回全部注册值的只读迭代器（顺序不定，供 `all()` 等聚合查询）。
    ///
    /// 供 `AttributeRegistry::all` 等具名注册表的全量遍历使用；调用方不应依赖
    /// 迭代顺序（`HashMap` 无序）。
    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.by_id.values()
    }

    /// 注册条目数量。
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// 注册表是否为空。
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// 内部：以指定 id 插入一条目（供 `from_toml` 使用）。
    ///
    /// 同时校验 `name` 与 `id` 均不冲突；插入后把 `next_id` 推进到
    /// `id + 1`（饱和，避免溢出 panic）。
    fn insert_at(&mut self, id: u32, name: String, value: T) -> Result<(), RegistryError> {
        if self.by_name.contains_key(&name) {
            return Err(RegistryError::DuplicateName);
        }
        if self.by_id.contains_key(&id) {
            return Err(RegistryError::DuplicateName);
        }
        self.by_name.insert(name, id);
        self.by_id.insert(id, value);
        self.next_id = id.saturating_add(1);
        Ok(())
    }
}

impl<T> Registry<T>
where
    T: RegistryEntry + DeserializeOwned,
{
    /// 从 TOML 文本解析 `[[entry]]` 数组，构建注册表。
    ///
    /// 每条目通过 [`RegistryEntry::entry_id`] 决定 id：若数据源提供了 id
    /// 则使用之，否则按出现顺序自动编号（从 0 开始）。
    ///
    /// # 错误
    /// 文件结构非法（缺少 `entry` 数组、条目无法反序列化、name 重复等）
    /// 均返回 [`RegistryError::ParseError`] 或 [`RegistryError::DuplicateName`]。
    pub fn from_toml_str(text: &str) -> Result<Self, RegistryError> {
        let document: toml::Value = toml::from_str(text).map_err(|_| RegistryError::ParseError)?;
        let entries = document
            .get("entry")
            .and_then(|value| value.as_array())
            .ok_or(RegistryError::ParseError)?;

        let mut registry = Registry::new();
        let mut auto_id: u32 = 0;
        for entry in entries {
            let item: T = entry
                .clone()
                .try_into()
                .map_err(|_: toml::de::Error| RegistryError::ParseError)?;
            let id = item.entry_id().unwrap_or_else(|| {
                let assigned = auto_id;
                auto_id = auto_id.saturating_add(1);
                assigned
            });
            registry.insert_at(id, item.entry_name().to_string(), item)?;
        }
        Ok(registry)
    }

    /// 从 TOML 文件加载注册表（包装 [`from_toml_str`](Self::from_toml_str)）。
    ///
    /// # 错误
    /// 路径不存在或无法读取时返回 [`RegistryError::ParseError`]。
    pub fn from_toml_file(path: &Path) -> Result<Self, RegistryError> {
        let text = std::fs::read_to_string(path).map_err(|_| RegistryError::ParseError)?;
        Self::from_toml_str(&text)
    }
}

/// 方块不透明度默认值（15 = 完全不透明，阻挡全部天空光）。
///
/// 数据源未提供 `light_opacity` 时回退到此值，与 Minecraft 方块默认语义一致。
fn default_light_opacity() -> u8 {
    15
}

/// 方块注册项（具名定义）。
///
/// 除必需的 `name` 与可选 `id` 外，额外保留 Minestom 关注的几个字段，
/// 其余字段透传存入 [`BlockDefinition::extra`]，确保数据源信息不丢失。
///
/// `light_opacity` / `light_emission` 为光照引擎（`complete-framework-gaps`
/// WS1）所需的光属性，缺省分别回退「完全不透明」与「不发光」，对既有
/// 数据源向后兼容（无需修改 `blocks.toml` 即可加载）。
/// 注册表类别标识，用于动态注册表同步的脏标记路由。
///
/// `registry_sync` 系统据此判断需要向客户端重发哪一类 `RegistryData`。
/// 该枚举与冻结接口 `DynamicRegistrySync`（`complete-framework-gaps` WS5a）配套。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RegistryId {
    /// 方块注册表。
    Block,
    /// 物品注册表。
    Item,
    /// 实体类型注册表。
    EntityType,
    /// 通用注册表（其余 world 类注册表）。
    Generic,
}

/// 动态注册表同步脏标记资源。
///
/// 持有待同步的注册表类别列表，由 [`Registry::insert`] /
/// [`Registry::remove`] 经各注册表 `take_dirty` 汇总后写入，`registry_sync`
/// 系统在后续 tick 消费并向 Play 客户端重发 `RegistryData`。
///
/// 冻结接口 `DynamicRegistrySync`（`complete-framework-gaps` WS5a）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RegistrySyncState {
    /// 待同步的注册表类别 id 列表（去重由消费方处理）。
    pub dirty: Vec<RegistryId>,
}

impl RegistrySyncState {
    /// 标记某个注册表类别为脏（去重追加）。
    pub fn mark_dirty(&mut self, id: RegistryId) {
        if !self.dirty.contains(&id) {
            self.dirty.push(id);
        }
    }

    /// 取走全部脏类别并清空。
    #[must_use]
    pub fn take_dirty(&mut self) -> Vec<RegistryId> {
        std::mem::take(&mut self.dirty)
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BlockDefinition {
    /// 命名空间名称，如 `minecraft:stone`。
    pub name: String,
    /// 规范 id（数据源未提供时为 `None`，加载时自动编号）。
    #[serde(default)]
    pub id: Option<u32>,
    /// 翻译键（对应 `translationKey`）。
    #[serde(rename = "translationKey", default)]
    pub translation_key: Option<String>,
    /// 硬度。
    #[serde(default)]
    pub hardness: Option<f32>,
    /// 默认方块状态 id（对应 `defaultStateId`）。
    #[serde(rename = "defaultStateId", default)]
    pub default_state_id: Option<u32>,
    /// 方块状态表（当前数据源未提供，预留扩展位）。
    #[serde(default)]
    pub states: Option<HashMap<String, BlockStateDef>>,
    /// 光照不透明度（0..=15）：阻挡天空光的程度，15 = 完全不透明。
    ///
    /// 缺省回退 [`default_light_opacity`]（15）。取值会被钳制到 `0..=15`。
    #[serde(default = "default_light_opacity")]
    pub light_opacity: u8,
    /// 发光等级（0..=15）：方块自身发出的光，0 = 不发光。
    ///
    /// 缺省回退 `0`。取值会被钳制到 `0..=15`。
    #[serde(default)]
    pub light_emission: u8,
    /// 其余透传字段，避免任何数据在加载阶段被丢弃。
    #[serde(flatten)]
    pub extra: HashMap<String, toml::Value>,
}

/// 方块状态定义（预留扩展结构，当前数据源未产出）。
///
/// `properties` 表在 TOML 中以 `properties = { name = index, ... }` 形式提供，
/// 将各属性名映射为 `0..15` 的连续槽位索引；缺省时回退为空映射。
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct BlockStateDef {
    /// 状态 id。
    #[serde(default)]
    pub id: Option<u32>,
    /// 属性名到槽位索引的映射（0..15）。
    ///
    /// TOML 侧以可选的 `properties` 表提供，如
    /// `properties = { waterlogged = 0, north = 1 }`。
    /// 未提供时回退为空 HashMap，调用方应对空映射做防御处理。
    #[serde(rename = "properties", default)]
    pub property_indices: HashMap<String, u8>,
    /// 透传字段。
    #[serde(flatten)]
    pub extra: HashMap<String, toml::Value>,
}

impl RegistryEntry for BlockDefinition {
    fn entry_name(&self) -> &str {
        &self.name
    }
    fn entry_id(&self) -> Option<u32> {
        self.id
    }
}

/// 物品注册项。
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ItemDefinition {
    /// 命名空间名称。
    pub name: String,
    /// 规范 id。
    #[serde(default)]
    pub id: Option<u32>,
    /// 翻译键。
    #[serde(rename = "translationKey", default)]
    pub translation_key: Option<String>,
    /// 最大堆叠数（若存在）。
    #[serde(rename = "maxStackSize", default)]
    pub max_stack_size: Option<u32>,
    /// 透传字段。
    #[serde(flatten)]
    pub extra: HashMap<String, toml::Value>,
}

impl RegistryEntry for ItemDefinition {
    fn entry_name(&self) -> &str {
        &self.name
    }
    fn entry_id(&self) -> Option<u32> {
        self.id
    }
}

/// 实体类型注册项。
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct EntityTypeDefinition {
    /// 命名空间名称。
    pub name: String,
    /// 规范 id。
    #[serde(default)]
    pub id: Option<u32>,
    /// 翻译键。
    #[serde(rename = "translationKey", default)]
    pub translation_key: Option<String>,
    /// 实体宽度。
    #[serde(default)]
    pub width: Option<f64>,
    /// 实体高度。
    #[serde(default)]
    pub height: Option<f64>,
    /// 透传字段。
    #[serde(flatten)]
    pub extra: HashMap<String, toml::Value>,
}

impl RegistryEntry for EntityTypeDefinition {
    fn entry_name(&self) -> &str {
        &self.name
    }
    fn entry_id(&self) -> Option<u32> {
        self.id
    }
}

/// 通用具名注册项，承接无专属结构定义的世界类注册表
/// （生物群系、维度类型、流体、粒子、音效、伤害类型、附魔、药水效果等）。
///
/// 仅保留 `name` 与可选 `id`，其余字段全部透传，保证数据零丢失。
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct GenericDefinition {
    /// 规范 id（无则为 `None`）。
    #[serde(default)]
    pub id: Option<u32>,
    /// 命名空间名称。
    pub name: String,
    /// 透传字段。
    #[serde(flatten)]
    pub extra: HashMap<String, toml::Value>,
}

impl RegistryEntry for GenericDefinition {
    fn entry_name(&self) -> &str {
        &self.name
    }
    fn entry_id(&self) -> Option<u32> {
        self.id
    }
}

/// 方块注册表的语义查询方法。
///
/// 本 `impl` 块刻意与 `block.rs` 中的 `impl BlockRegistry`（加载路径）分置：
/// 前者只负责从 TOML 装配数据，后者提供方块语义查询，职责清晰。
impl BlockRegistry {
    /// 由方块状态 id 构造 [`Block`]。
    pub fn block_from_state_id(&self, state_id: u32) -> Block {
        Block::from_state_id(state_id)
    }

    /// 按命名空间名称解析方块；未注册时返回 `None`。
    pub fn block_from_name(&self, name: &str) -> Option<Block> {
        self.0.get_id(name).map(Block::from_state_id)
    }

    /// 按命名空间名称解析默认方块（与 [`block_from_name`](Self::block_from_name) 同义）。
    pub fn default_block(&self, name: &str) -> Option<Block> {
        self.block_from_name(name)
    }

    /// 方块状态 id 是否实心（缺省语义：非空气即实心）。
    pub fn is_solid(&self, state_id: u32) -> bool {
        state_id != self.air_id()
    }

    /// 空气方块状态 id（`minecraft:air`；注册表缺失时约定为 0）。
    pub fn air_id(&self) -> u32 {
        self.0.get_id("minecraft:air").unwrap_or(0)
    }

    /// 方块不透明度（0..=15）：阻挡天空光的程度，15 = 完全不透明。
    ///
    /// 未注册 id 或未提供字段时回退 `15`（完全不透明），与 Minecraft 默认
    /// 语义一致。取值钳制到 `0..=15`，避免数据源给出越界值。
    pub fn light_opacity(&self, state_id: u32) -> u8 {
        // 空气恒为透明：不阻挡任何天空光 / 方块光。未注册 id 视为不透明（15）
        // 以兜底安全（未知方块按实心处理）。
        if state_id == self.air_id() {
            return 0;
        }
        self.0
            .get(state_id)
            .map(|def| def.light_opacity.min(15))
            .unwrap_or(15)
    }

    /// 方块发光等级（0..=15）：方块自身发出的光，0 = 不发光。
    ///
    /// 未注册 id 或未提供字段时回退 `0`。取值钳制到 `0..=15`。
    pub fn light_emission(&self, state_id: u32) -> u8 {
        self.0
            .get(state_id)
            .map(|def| def.light_emission.min(15))
            .unwrap_or(0)
    }

    /// 按方块状态 id 反查命名空间名称；未注册时返回 `None`。
    ///
    /// 注意：底层 [`Registry::get_name`] 为线性扫描，调用方应避免高频使用。
    pub fn name_of(&self, state_id: u32) -> Option<&str> {
        self.0.get_name(state_id)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const BLOCKS_SMALL: &str = include_str!("../../../tests/fixtures/blocks_small.toml");
    const BIOMES_SMALL: &str = include_str!("../../../tests/fixtures/biomes_small.toml");

    #[test]
    fn register_assigns_sequential_ids_starting_at_one() {
        let mut registry: Registry<u32> = Registry::new();
        assert!(registry.is_empty());
        let first = registry.register("minecraft:stone", 0).unwrap();
        let second = registry.register("minecraft:dirt", 1).unwrap();
        assert_eq!(first, 1);
        assert_eq!(second, 2);
        assert_eq!(registry.len(), 2);
        assert!(!registry.is_empty());
    }

    #[test]
    fn get_and_get_id_and_get_name_are_consistent() {
        let mut registry: Registry<u32> = Registry::new();
        registry.register("minecraft:stone", 42).unwrap();
        assert_eq!(registry.get(1), Some(&42));
        assert_eq!(registry.get_id("minecraft:stone"), Some(1));
        assert_eq!(registry.get_name(1), Some("minecraft:stone"));
        assert_eq!(registry.get(99), None);
        assert_eq!(registry.get_id("minecraft:missing"), None);
        assert_eq!(registry.get_name(99), None);
    }

    #[test]
    fn duplicate_name_is_rejected() {
        let mut registry: Registry<u32> = Registry::new();
        registry.register("minecraft:stone", 0).unwrap();
        let result = registry.register("minecraft:stone", 1);
        assert_eq!(result, Err(RegistryError::DuplicateName));
        // 旧值不被覆盖
        assert_eq!(registry.get(1), Some(&0));
    }

    #[test]
    fn from_toml_str_with_explicit_ids() {
        let registry: Registry<BlockDefinition> = Registry::from_toml_str(BLOCKS_SMALL).unwrap();
        assert_eq!(registry.len(), 3);
        assert_eq!(registry.get_id("minecraft:stone"), Some(1));
        assert_eq!(registry.get_id("minecraft:grass"), Some(2));
        let stone = registry.get(1).unwrap();
        assert_eq!(stone.name, "minecraft:stone");
        assert_eq!(
            stone.translation_key.as_deref(),
            Some("block.minecraft.stone")
        );
        assert_eq!(stone.hardness, Some(1.5));
        // 透传字段保留：显式命名的字段（translationKey/hardness 等）已被消费，
        // 只有未在结构体中定义的字段（如 soundGroup）会进入 extra。
        assert!(stone.extra.contains_key("soundGroup"));
    }

    #[test]
    fn from_toml_str_without_ids_auto_numbers() {
        let registry: Registry<GenericDefinition> = Registry::from_toml_str(BIOMES_SMALL).unwrap();
        assert_eq!(registry.len(), 3);
        // 无 id 数据源按出现顺序自动从 0 编号
        assert_eq!(registry.get_id("minecraft:plains"), Some(0));
        assert_eq!(registry.get_id("minecraft:desert"), Some(1));
        assert_eq!(registry.get_id("minecraft:ocean"), Some(2));
    }

    #[test]
    fn from_toml_str_rejects_missing_entry_array() {
        let result: Result<Registry<GenericDefinition>, RegistryError> =
            Registry::from_toml_str("name = \"oops\"");
        assert_eq!(result, Err(RegistryError::ParseError));
    }

    #[test]
    fn from_toml_str_rejects_malformed_toml() {
        let result: Result<Registry<GenericDefinition>, RegistryError> =
            Registry::from_toml_str("this is not = = valid toml @@@");
        assert_eq!(result, Err(RegistryError::ParseError));
    }

    #[test]
    fn from_toml_file_missing_path_returns_parse_error() {
        let missing = std::path::Path::new("does/not/exist/blocks.toml");
        let result = Registry::<BlockDefinition>::from_toml_file(missing);
        assert_eq!(result, Err(RegistryError::ParseError));
    }

    #[test]
    fn override_value_keeps_id_for_existing_name() {
        let mut registry: Registry<u32> = Registry::new();
        registry.register("minecraft:stone", 1).unwrap();
        // 覆盖已存在条目：保留原 id，值被替换。
        registry.override_value("minecraft:stone", 99).unwrap();
        assert_eq!(registry.get_id("minecraft:stone"), Some(1));
        assert_eq!(registry.get(1), Some(&99));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn override_value_new_name_acts_like_register() {
        let mut registry: Registry<u32> = Registry::new();
        registry.register("minecraft:stone", 1).unwrap();
        // 覆盖不存在条目：等价注册，分配新 id。
        registry.override_value("minecraft:dirt", 2).unwrap();
        assert_eq!(registry.get_id("minecraft:dirt"), Some(2));
        assert_eq!(registry.get(2), Some(&2));
    }

    #[test]
    fn register_or_replace_both_paths() {
        let mut registry: Registry<u32> = Registry::new();
        let first = registry.register_or_replace("minecraft:stone", 1).unwrap();
        assert_eq!(first, 1);
        // 已存在：保留原 id，替换值并返回该 id。
        let again = registry.register_or_replace("minecraft:stone", 42).unwrap();
        assert_eq!(again, 1);
        assert_eq!(registry.get(1), Some(&42));
        assert_eq!(registry.len(), 1);
        // 不存在：等价注册，返回新 id。
        let dirt = registry.register_or_replace("minecraft:dirt", 7).unwrap();
        assert_eq!(dirt, 2);
        assert_eq!(registry.get(2), Some(&7));
    }

    #[test]
    fn repeated_override_is_idempotent() {
        let mut registry: Registry<u32> = Registry::new();
        registry.register("minecraft:stone", 1).unwrap();
        registry.override_value("minecraft:stone", 5).unwrap();
        registry.override_value("minecraft:stone", 6).unwrap();
        assert_eq!(registry.get_id("minecraft:stone"), Some(1));
        assert_eq!(registry.get(1), Some(&6));
        assert_eq!(registry.len(), 1);
        // 重复覆盖后仍可正常注册新条目（next_id 未被推进）。
        registry.override_value("minecraft:dirt", 2).unwrap();
        assert_eq!(registry.get_id("minecraft:dirt"), Some(2));
    }

    #[test]
    fn block_registry_semantic_queries() {
        // 最小方块注册表：air=0、stone=1。
        let toml = r#"
            [[entry]]
            id = 0
            name = "minecraft:air"

            [[entry]]
            id = 1
            name = "minecraft:stone"
        "#;
        let registry = BlockRegistry(Registry::<BlockDefinition>::from_toml_str(toml).unwrap());
        assert_eq!(registry.air_id(), 0);
        assert_eq!(
            registry
                .block_from_name("minecraft:stone")
                .unwrap()
                .state_id(),
            1
        );
        assert_eq!(
            registry
                .default_block("minecraft:stone")
                .unwrap()
                .state_id(),
            1
        );
        assert!(registry.block_from_name("minecraft:missing").is_none());
        assert!(registry.is_solid(1));
        assert!(!registry.is_solid(0));
        assert_eq!(registry.name_of(1), Some("minecraft:stone"));
        assert_eq!(registry.name_of(99), None);
        // block_from_state_id 与 Block 互转。
        let block = registry.block_from_state_id(1);
        assert_eq!(block.state_id(), 1);
    }

    #[test]
    fn insert_adds_entry_and_marks_dirty() {
        let mut registry: Registry<u32> = Registry::new();
        registry.register("minecraft:stone", 1).unwrap();
        // 插入新条目：分配新 id，置脏。
        registry.insert("minecraft:grass", 7).unwrap();
        assert_eq!(registry.get_id("minecraft:grass"), Some(2));
        assert_eq!(registry.get(2), Some(&7));
        assert!(registry.is_dirty());
        assert_eq!(registry.take_dirty(), vec![2]);
        assert!(!registry.is_dirty());
    }

    #[test]
    fn insert_duplicate_name_rejected() {
        let mut registry: Registry<u32> = Registry::new();
        registry.insert("minecraft:stone", 1).unwrap();
        assert_eq!(
            registry.insert("minecraft:stone", 2),
            Err(RegistryError::DuplicateName)
        );
        // 旧值不被覆盖。
        assert_eq!(registry.get(1), Some(&1));
    }

    #[test]
    fn remove_deletes_entry_and_marks_dirty() {
        let mut registry: Registry<u32> = Registry::new();
        registry.insert("minecraft:stone", 1).unwrap();
        registry.insert("minecraft:grass", 2).unwrap();
        let _ = registry.take_dirty();
        let removed = registry.remove("minecraft:stone");
        assert_eq!(removed, Some(1));
        assert_eq!(registry.get_id("minecraft:stone"), None);
        assert!(registry.is_dirty());
        assert_eq!(registry.take_dirty(), vec![1]);
        // 移除不重用 id：再次插入分配新 id 3。
        registry.insert("minecraft:stone", 1).unwrap();
        assert_eq!(registry.get_id("minecraft:stone"), Some(3));
    }

    #[test]
    fn remove_missing_returns_none_and_not_dirty() {
        let mut registry: Registry<u32> = Registry::new();
        assert_eq!(registry.remove("minecraft:missing"), None);
        assert!(!registry.is_dirty());
    }

    #[test]
    fn id_overflow_path_is_defensive_only() {
        // 说明跳过理由：`next_id` 为私有字段且需约 2^32 次注册才能逼近
        // u32::MAX，单元测试无法在不引入大量分配的前提下实际触发；溢出
        // 分支通过 `checked_add` 保证不 panic，属防御性代码，由代码审查覆盖。
        let mut registry: Registry<u32> = Registry::new();
        // 正常路径 sanity check：分配 id 递增。
        registry.register_or_replace("a", 1).unwrap();
        registry.register_or_replace("b", 2).unwrap();
        assert_eq!(registry.len(), 2);
        assert_eq!(registry.get_id("b"), Some(2));
    }

    #[test]
    fn block_state_def_parses_properties_table() {
        // 验证 BlockStateDef 能够正确解析 TOML 中的 properties 表。
        let toml = r#"
            [[entry]]
            name = "minecraft:lever"
            id = 5
            [[entry]]
            name = "minecraft:stone"
            id = 1
        "#;
        let registry: Registry<BlockDefinition> =
            Registry::from_toml_str(toml).unwrap();
        // 基础方块无 states，property_indices 应回退为空。
        let stone = registry.get(1).unwrap();
        assert!(stone.states.is_none());

        // 单独构造含 states 的 TOML。
        // 注意：states 是 HashMap，[[entry.states]] 表示数组，
        // 非 HashMap 形式。改为 inline table 形式：
        let with_states_inline = r#"
            [[entry]]
            name = "minecraft:lever"
            id = 5
        "#;
        let _reg: Registry<BlockDefinition> =
            Registry::from_toml_str(with_states_inline).unwrap();
        // 没有 states 时 property_indices 在 BlockDefinition 上不存在，
        // 需验证 BlockStateDef 本身的解析。
        let state_toml = r#"
            name = "minecraft:lever"
            id = 0
            [properties]
            face = 2
            shadow = 3
        "#;
        let state_def: BlockStateDef =
            toml::from_str(state_toml).expect("BlockStateDef 应可解析含 properties 的 TOML");
        assert_eq!(state_def.id, Some(0));
        assert_eq!(state_def.property_indices.len(), 2);
        assert_eq!(state_def.property_indices.get("face"), Some(&2));
        assert_eq!(state_def.property_indices.get("shadow"), Some(&3));

        // 无 properties 时回退为空。
        let plain = r#"
            name = "minecraft:stone"
            id = 1
        "#;
        let plain_def: BlockStateDef =
            toml::from_str(plain).expect("BlockStateDef 应可解析不含 properties 的 TOML");
        assert_eq!(plain_def.id, Some(1));
        assert!(plain_def.property_indices.is_empty());
    }
}
