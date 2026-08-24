//! 附魔承载与注册表（R11）。
//!
//! [`Enchantment`] 表示一个具名附魔条目（类型 + 注册表序位 + 应用等级）；
//! [`EnchantmentList`] 承载附魔集合，与既有 enchantments(13) Nbt 线格式
//! （`Compound { "enchantments": List[ { "id": String, "lvl": Short } ] }`）
//! 互转，供 [`ItemStack::enchantments`] / [`ItemStack::set_enchantments`] 使用。
//!
//! [`EnchantmentRegistry`] 为数据驱动注册表：从 `resources/data/enchantments.toml`
//! 加载（`[[entry]]` 数组，无显式 id，按出现顺序从 0 编号），id 序位对齐
//! Java `Enchantments.java`（vanilla 1.21.11 enchantment registry 序位，
//! sharpness=32 / power=22 / protection=12 等）。本注册表以 `HashMap` 承载
//! 「id → 条目」双向映射，独立于 `registries` 中的通用
//! `Registry<GenericDefinition>` 世界类注册表。
//!
//! 变更标识符：`complete-missing-subsystems`（R11 item 子包行为 API）。

use std::collections::HashMap;
use std::path::Path;

use crate::item_stack::{ComponentValue, ItemStack};
use crate::protocol::nbt::NbtTag;
use crate::resource::registries::RegistryError;

/// 附魔线格式中承载附魔列表的 Nbt 键名（enchantments=13）。
const KEY_ENCHANTMENTS: &str = "enchantments";
/// 单条附魔 Nbt 中类型名键（vanilla EnchantmentList 的 key）。
const KEY_ID: &str = "id";
/// 单条附魔 Nbt 中等级键（vanilla EnchantmentList 的 value）。
const KEY_LVL: &str = "lvl";

/// 单个附魔条目（类型 + 注册表序位 + 应用等级）。
///
/// `id` 为该附魔类型在注册表中的序位（对齐 Java `Enchantments.java`），
/// 仅供注册表查询/测试断言使用——enchantments Nbt 线格式只承载名称字符串，
/// 不承载 id（`[`EnchantmentList::from_nbt`]` 解析时置 0）。
#[derive(Clone, Debug, PartialEq)]
pub struct Enchantment {
    /// 命名空间名称，如 `minecraft:sharpness`。
    pub name: String,
    /// 注册表序位（线格式不承载；`Enchantment::new` 默认 0）。
    pub id: u32,
    /// 附魔等级。
    pub level: u32,
}

impl Enchantment {
    /// 以名称与等级构造（id 暂为 0，需经注册表解析时用 [`with_id`](Self::with_id) 补齐）。
    pub fn new(name: impl Into<String>, level: u32) -> Self {
        Enchantment {
            name: name.into(),
            id: 0,
            level,
        }
    }

    /// 链式设置注册表序位。
    pub fn with_id(mut self, id: u32) -> Self {
        self.id = id;
        self
    }
}

/// 附魔集合承载：经 enchantments(13) Nbt 线格式与组件互转。
#[derive(Clone, Debug, PartialEq)]
pub struct EnchantmentList {
    /// 附魔条目（保持加入顺序）。
    pub enchantments: Vec<Enchantment>,
}

impl EnchantmentList {
    /// 空附魔列表常量。
    pub const EMPTY: EnchantmentList = EnchantmentList {
        enchantments: Vec::new(),
    };

    /// 构造空列表。
    pub fn new() -> Self {
        EnchantmentList::EMPTY
    }

    /// 是否不包含任何附魔。
    pub fn is_empty(&self) -> bool {
        self.enchantments.is_empty()
    }

    /// 附魔条目数量。
    pub fn len(&self) -> usize {
        self.enchantments.len()
    }

    /// 追加一个附魔条目。
    pub fn push(&mut self, enchantment: Enchantment) {
        self.enchantments.push(enchantment);
    }

    /// 按名称查询指定附魔的等级；不存在时返回 `None`。
    pub fn level_of(&self, name: &str) -> Option<u32> {
        self.enchantments
            .iter()
            .find(|e| e.name == name)
            .map(|e| e.level)
    }

    /// 编码为 enchantments=13 线格式承载（`ComponentValue::Nbt`）。
    ///
    /// Nbt 结构：`Compound { "enchantments": List[ Compound { "id": String,
    /// "lvl": Short } ] }`。等级超过 `i16` 范围时裁剪到 `i16::MAX`（防御性，
    /// vanilla 附魔等级上限远低于该值）。
    pub fn to_component_value(&self) -> ComponentValue {
        let list: Vec<NbtTag> = self
            .enchantments
            .iter()
            .map(|e| {
                NbtTag::Compound(vec![
                    (KEY_ID.to_string(), NbtTag::String(e.name.clone())),
                    (
                        KEY_LVL.to_string(),
                        NbtTag::Short(i16::try_from(e.level).unwrap_or(i16::MAX)),
                    ),
                ])
            })
            .collect();
        ComponentValue::Nbt(NbtTag::Compound(vec![(
            KEY_ENCHANTMENTS.to_string(),
            NbtTag::List(list),
        )]))
    }

    /// 从既有 Nbt 承载解析附魔列表（与 [`to_component_value`](Self::to_component_value)
    /// 及 T1 框架约定的 enchantments=13 线格式互逆）。
    ///
    /// 解析规则：顶层 `Compound` 的 `"enchantments"` 键必须为 `List`，每个元素为
    /// `Compound`，含 `"id"`（String，必填）与 `"lvl"`（Short/Int，宽容解析，
    /// 负值按 0 处理）。结构不匹配返回 `None`。`id` 字段线格式不承载，置 0。
    pub fn from_nbt(tag: &NbtTag) -> Option<EnchantmentList> {
        let entries = match tag {
            NbtTag::Compound(e) => e.as_slice(),
            _ => return None,
        };
        let list = entries
            .iter()
            .find(|(k, _)| k.as_str() == KEY_ENCHANTMENTS)?;
        let items = match &list.1 {
            NbtTag::List(items) => items,
            _ => return None,
        };
        let mut enchantments = Vec::with_capacity(items.len());
        for item in items {
            let item_entries = match item {
                NbtTag::Compound(e) => e.as_slice(),
                _ => return None,
            };
            let id_entry = item_entries.iter().find(|(k, _)| k.as_str() == KEY_ID)?;
            let name = match &id_entry.1 {
                NbtTag::String(s) => s.clone(),
                _ => return None,
            };
            let lvl_entry = item_entries.iter().find(|(k, _)| k.as_str() == KEY_LVL)?;
            let level = match &lvl_entry.1 {
                NbtTag::Short(s) => u32::try_from(*s).unwrap_or(0),
                NbtTag::Int(i) => u32::try_from(*i).unwrap_or(0),
                _ => return None,
            };
            enchantments.push(Enchantment { name, id: 0, level });
        }
        Some(EnchantmentList { enchantments })
    }
}

impl Default for EnchantmentList {
    fn default() -> Self {
        Self::new()
    }
}

/// `ItemStack` 的附魔承载 getter/setter（经 enchantments=13 组件）。
impl ItemStack {
    /// 读取附魔列表；无 enchantments(13) 组件或结构不匹配时返回 `None`。
    pub fn enchantments(&self) -> Option<EnchantmentList> {
        match self.components.get(13) {
            Some(ComponentValue::Nbt(tag)) => EnchantmentList::from_nbt(tag),
            _ => None,
        }
    }

    /// 设置附魔列表（写入 enchantments=13 组件）。
    ///
    /// 使用 [`ItemComponents::set_at`] 按线格式 id=13 写入，与 custom_data(0)
    /// 的 Nbt 承载互不覆盖。
    pub fn set_enchantments(&mut self, list: &EnchantmentList) {
        self.components.set_at(13, list.to_component_value());
    }
}

/// 附魔注册表（具名 `Resource`，数据驱动）。
///
/// 从 `resources/data/enchantments.toml`（`[[entry]]` 数组）加载；条目无显式
/// `id`，按出现顺序从 0 编号，序位对齐 Java `Enchantments.java` 常量清单。
/// 目录缺失或解析失败返回 [`RegistryError`]，由调用方决定回退为空表。
#[derive(Default, Debug, Clone)]
pub struct EnchantmentRegistry {
    /// id → 附魔条目。
    by_id: HashMap<u32, Enchantment>,
    /// name → id（双向查询）。
    by_name: HashMap<String, u32>,
}

impl EnchantmentRegistry {
    /// 从 TOML 文本加载附魔注册表（主要供单元测试使用）。
    ///
    /// # 错误
    /// 文本非法、缺少 `entry` 数组或条目缺少 `name` 字段时返回
    /// [`RegistryError::ParseError`]。
    pub fn from_toml_str(text: &str) -> Result<Self, RegistryError> {
        let document: toml::Value = toml::from_str(text).map_err(|_| RegistryError::ParseError)?;
        let entries = document
            .get("entry")
            .and_then(|value| value.as_array())
            .ok_or(RegistryError::ParseError)?;
        let mut registry = EnchantmentRegistry::default();
        for (index, entry) in entries.iter().enumerate() {
            let name = entry
                .get("name")
                .and_then(|value| value.as_str())
                .ok_or(RegistryError::ParseError)?
                .to_string();
            let id = u32::try_from(index).map_err(|_| RegistryError::ParseError)?;
            registry.by_id.insert(
                id,
                Enchantment {
                    name: name.clone(),
                    id,
                    level: 0,
                },
            );
            registry.by_name.insert(name, id);
        }
        Ok(registry)
    }

    /// 从 TOML 文件加载附魔注册表。
    ///
    /// # 错误
    /// 路径不存在或无法读取时返回 [`RegistryError::ParseError`]。
    pub fn from_toml_file(path: &Path) -> Result<Self, RegistryError> {
        let text = std::fs::read_to_string(path).map_err(|_| RegistryError::ParseError)?;
        Self::from_toml_str(&text)
    }

    /// 按注册表序位查询附魔条目。
    pub fn by_id(&self, id: u32) -> Option<&Enchantment> {
        self.by_id.get(&id)
    }

    /// 按命名空间名称查询附魔条目。
    pub fn by_name(&self, name: &str) -> Option<&Enchantment> {
        self.by_name.get(name).and_then(|id| self.by_id.get(id))
    }

    /// 遍历全部附魔条目（顺序不定，供全量聚合查询）。
    pub fn all(&self) -> impl Iterator<Item = &Enchantment> {
        self.by_id.values()
    }

    /// 已注册附魔条目数量。
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// 注册表是否为空。
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn sharpness(level: u32) -> Enchantment {
        Enchantment::new("minecraft:sharpness", level).with_id(32)
    }

    #[test]
    fn new_and_accessors() {
        let e = Enchantment::new("minecraft:power", 3).with_id(22);
        assert_eq!(e.name, "minecraft:power");
        assert_eq!(e.id, 22);
        assert_eq!(e.level, 3);
        // 不调 with_id 时 id 默认为 0（线格式不承载 id，由注册表解析补齐）。
        assert_eq!(Enchantment::new("minecraft:unbreaking", 1).id, 0);
    }

    #[test]
    fn list_empty_and_push() {
        let mut list = EnchantmentList::new();
        assert!(list.is_empty());
        assert_eq!(list.len(), 0);
        list.push(sharpness(5));
        list.push(Enchantment::new("minecraft:power", 3).with_id(22));
        assert!(!list.is_empty());
        assert_eq!(list.len(), 2);
        assert_eq!(list.level_of("minecraft:sharpness"), Some(5));
        assert_eq!(list.level_of("minecraft:missing"), None);
    }

    #[test]
    fn list_nbt_roundtrip() {
        // 与 T1 框架约定的 enchantments=13 线格式一致：
        // Compound { "enchantments": List[ { "id": String, "lvl": Short } ] }。
        let mut list = EnchantmentList::new();
        list.push(sharpness(5));
        list.push(Enchantment::new("minecraft:power", 3));
        let tag = match list.to_component_value() {
            ComponentValue::Nbt(tag) => tag,
            _ => panic!("to_component_value 应产生 Nbt 承载"),
        };
        // 逐层断言结构（避免裸索引）。
        let entries = match &tag {
            NbtTag::Compound(e) => e,
            _ => panic!("顶层应为 Compound"),
        };
        assert!(entries.iter().any(|(k, _)| k.as_str() == "enchantments"));
        let list_tag = entries
            .iter()
            .find(|(k, _)| k.as_str() == "enchantments")
            .map(|(_, v)| v)
            .unwrap();
        let items = match list_tag {
            NbtTag::List(items) => items,
            _ => panic!("enchantments 应为 List"),
        };
        assert_eq!(items.len(), 2);
        let first = match items.first().unwrap() {
            NbtTag::Compound(e) => e,
            _ => panic!("元素应为 Compound"),
        };
        assert_eq!(
            first
                .iter()
                .find(|(k, _)| k.as_str() == "id")
                .map(|(_, v)| v),
            Some(&NbtTag::String("minecraft:sharpness".to_string()))
        );
        assert_eq!(
            first
                .iter()
                .find(|(k, _)| k.as_str() == "lvl")
                .map(|(_, v)| v),
            Some(&NbtTag::Short(5))
        );
        // roundtrip：from_nbt 还原 name/level；id 线格式不承载，为 0。
        let back = EnchantmentList::from_nbt(&tag).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back.level_of("minecraft:sharpness"), Some(5));
        assert_eq!(back.level_of("minecraft:power"), Some(3));
        assert_eq!(back.enchantments[0].name, "minecraft:sharpness");
        assert_eq!(back.enchantments[0].id, 0);
    }

    #[test]
    fn from_nbt_tolerates_int_lvl_and_negative() {
        // 宽容解析：lvl 可为 Int（非 Short 旧承载）；负值按 0 处理。
        let tag = NbtTag::Compound(vec![(
            "enchantments".to_string(),
            NbtTag::List(vec![
                NbtTag::Compound(vec![
                    (
                        "id".to_string(),
                        NbtTag::String("minecraft:power".to_string()),
                    ),
                    ("lvl".to_string(), NbtTag::Int(3)),
                ]),
                NbtTag::Compound(vec![
                    (
                        "id".to_string(),
                        NbtTag::String("minecraft:curse".to_string()),
                    ),
                    ("lvl".to_string(), NbtTag::Short(-1)),
                ]),
            ]),
        )]);
        let list = EnchantmentList::from_nbt(&tag).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list.level_of("minecraft:power"), Some(3));
        assert_eq!(list.level_of("minecraft:curse"), Some(0));
    }

    #[test]
    fn from_nbt_rejects_malformed_shapes() {
        // 非 Compound 输入。
        assert!(EnchantmentList::from_nbt(&NbtTag::Int(1)).is_none());
        // 缺 "enchantments" 键。
        assert!(EnchantmentList::from_nbt(&NbtTag::Compound(vec![])).is_none());
        // enchantments 不是 List。
        let not_list = NbtTag::Compound(vec![("enchantments".to_string(), NbtTag::Int(0))]);
        assert!(EnchantmentList::from_nbt(&not_list).is_none());
        // 元素缺 id 或 lvl。
        let bad_item = NbtTag::Compound(vec![(
            "enchantments".to_string(),
            NbtTag::List(vec![NbtTag::Compound(vec![(
                "lvl".to_string(),
                NbtTag::Short(1),
            )])]),
        )]);
        assert!(EnchantmentList::from_nbt(&bad_item).is_none());
        // id 类型不是 String。
        let bad_id = NbtTag::Compound(vec![(
            "enchantments".to_string(),
            NbtTag::List(vec![NbtTag::Compound(vec![(
                "id".to_string(),
                NbtTag::Int(0),
            )])]),
        )]);
        assert!(EnchantmentList::from_nbt(&bad_id).is_none());
    }

    #[test]
    fn level_overflow_clamps_to_i16_max() {
        // 防御：等级超 i16 范围时裁剪，编码不失败。
        let list = EnchantmentList {
            enchantments: vec![Enchantment::new("minecraft:sharpness", u32::MAX)],
        };
        let tag = match list.to_component_value() {
            ComponentValue::Nbt(tag) => tag,
            _ => panic!("应为 Nbt"),
        };
        let back = EnchantmentList::from_nbt(&tag).unwrap();
        assert_eq!(
            back.level_of("minecraft:sharpness"),
            Some(u32::try_from(i16::MAX).unwrap())
        );
    }

    #[test]
    fn item_stack_enchantments_roundtrip() {
        let mut list = EnchantmentList::new();
        list.push(sharpness(5));
        let mut item = ItemStack::new(264, 1);
        item.set_enchantments(&list);
        assert_eq!(
            item.enchantments().unwrap().level_of("minecraft:sharpness"),
            Some(5)
        );
        // 与 custom_data(0) 互不覆盖：get(13) 有值而 get(0) 为 None。
        assert!(item.components.get(13).is_some());
        assert!(item.components.get(0).is_none());
        // 未设置时返回 None。
        assert!(ItemStack::new(264, 1).enchantments().is_none());
    }

    #[test]
    fn registry_loads_from_toml_and_queries() {
        let toml = r#"
            [[entry]]
            name = "minecraft:depth_strider"

            [[entry]]
            name = "minecraft:protection"

            [[entry]]
            name = "minecraft:sharpness"
        "#;
        let registry = EnchantmentRegistry::from_toml_str(toml).unwrap();
        assert_eq!(registry.len(), 3);
        // 无显式 id 数据源按出现顺序从 0 编号（对齐 Enchantments.java 序位）。
        assert_eq!(registry.by_name("minecraft:depth_strider").unwrap().id, 0);
        assert_eq!(registry.by_name("minecraft:protection").unwrap().id, 1);
        assert_eq!(registry.by_name("minecraft:sharpness").unwrap().id, 2);
        assert_eq!(registry.by_id(2).unwrap().name, "minecraft:sharpness");
        assert!(registry.by_id(99).is_none());
        assert!(registry.by_name("minecraft:missing").is_none());
    }

    #[test]
    fn registry_rejects_malformed_toml() {
        // 非 TOML 文本。
        assert!(matches!(
            EnchantmentRegistry::from_toml_str("this is not = = valid toml @@@"),
            Err(RegistryError::ParseError)
        ));
        // 缺 entry 数组。
        assert!(matches!(
            EnchantmentRegistry::from_toml_str("name = \"x\""),
            Err(RegistryError::ParseError)
        ));
        // 条目缺 name。
        assert!(matches!(
            EnchantmentRegistry::from_toml_str("[[entry]]\nlevel = 3"),
            Err(RegistryError::ParseError)
        ));
        // 文件缺失。
        let missing = Path::new("does/not/exist/enchantments.toml");
        assert!(matches!(
            EnchantmentRegistry::from_toml_file(missing),
            Err(RegistryError::ParseError)
        ));
    }

    #[test]
    fn registry_loads_real_data_file() {
        // 真实注册数据：43 项，序位对齐 Java Enchantments.java。
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/data/enchantments.toml");
        let registry = EnchantmentRegistry::from_toml_file(&path).unwrap();
        assert_eq!(registry.len(), 43);
        assert_eq!(registry.by_name("minecraft:sharpness").unwrap().id, 32);
        assert_eq!(registry.by_name("minecraft:power").unwrap().id, 22);
        assert_eq!(registry.by_name("minecraft:protection").unwrap().id, 12);
        assert_eq!(registry.by_name("minecraft:depth_strider").unwrap().id, 0);
        assert_eq!(registry.by_name("minecraft:unbreaking").unwrap().id, 42);
    }
}
