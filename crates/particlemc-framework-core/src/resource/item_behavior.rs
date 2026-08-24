// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 物品行为承载与值类型（R11）。
//!
//! 书承载：[`WritableBook`] / [`WrittenBook`] 经组件 id
//! `writable_book_content`(52) / `written_book_content`(53)（以 Nbt 承载，
//! 页/标题文本经 [`crate::text_component::Component`]↔NBT）与 [`ItemStack`]
//! 互转，见 [`ItemStack::writable_book`] / [`ItemStack::written_book`]。
//! 组件 id 对齐 Java `DataComponents.java`（已核实：`writable_book_content` =
//! 52、`written_book_content` = 53）。
//!
//! 行为值类型：[`Armor`]（护甲/韧性）、[`Weapon`]（攻击伤害/攻速）、
//! [`Tool`]（挖掘速度/等级）、[`Crossbow`]（蓄能状态/装填弹丸）。以纯值类型 +
//! 构造/访问器实现（不接包），对齐 Java `item/armor`、`item/weapon`、
//! `item/tool`、`item/crossbow` 子包语义。
//!
//! 变更标识符：`complete-missing-subsystems`（R11 item 子包行为 API）。

use crate::item_stack::{ComponentValue, ItemStack};
use crate::protocol::nbt::NbtTag;
use crate::text_component::Component;

/// 可写书承载（对应 `writable_book_content`(52) 组件）。
///
/// Nbt 承载格式：`Compound { "pages": List[ Component-nbt ] }`（页经
/// [`Component::to_nbt`] 序列化），键名对齐 vanilla `WritableBookContent` CODEC。
#[derive(Clone, Debug, PartialEq)]
pub struct WritableBook {
    /// 书页（每页一个文本组件）。
    pub pages: Vec<Component>,
}

impl WritableBook {
    /// 以页集合构造。
    pub fn new(pages: Vec<Component>) -> Self {
        WritableBook { pages }
    }

    /// 书页（只读切片）。
    pub fn pages(&self) -> &[Component] {
        &self.pages
    }

    /// 序列化为 Nbt 承载（`Compound { "pages": List }`）。
    pub fn to_nbt(&self) -> NbtTag {
        NbtTag::Compound(vec![("pages".to_string(), component_list_nbt(&self.pages))])
    }

    /// 从 Nbt 承载解析；结构不匹配返回 `None`。
    pub fn from_nbt(tag: &NbtTag) -> Option<WritableBook> {
        let entries = compound_entries(tag)?;
        let pages = component_list_from_nbt(entries, "pages")?;
        Some(WritableBook { pages })
    }
}

/// 已写书承载（对应 `written_book_content`(53) 组件）。
///
/// Nbt 承载格式：`Compound { "title": Component-nbt, "author": String,
/// "pages": List[ Component-nbt ] }`，键名对齐 vanilla `WrittenBookContent`
/// CODEC（`generation` / `resolved` 等字段不在本任务范围）。
#[derive(Clone, Debug, PartialEq)]
pub struct WrittenBook {
    /// 书名（文本组件）。
    pub title: Component,
    /// 作者名。
    pub author: String,
    /// 书页（每页一个文本组件）。
    pub pages: Vec<Component>,
}

impl WrittenBook {
    /// 以标题、作者与页集合构造。
    pub fn new(title: Component, author: String, pages: Vec<Component>) -> Self {
        WrittenBook {
            title,
            author,
            pages,
        }
    }

    /// 书名（只读引用）。
    pub fn title(&self) -> &Component {
        &self.title
    }

    /// 作者名（只读引用）。
    pub fn author(&self) -> &str {
        &self.author
    }

    /// 书页（只读切片）。
    pub fn pages(&self) -> &[Component] {
        &self.pages
    }

    /// 序列化为 Nbt 承载（`Compound { "title", "author", "pages" }`）。
    pub fn to_nbt(&self) -> NbtTag {
        NbtTag::Compound(vec![
            ("title".to_string(), self.title.to_nbt()),
            ("author".to_string(), NbtTag::String(self.author.clone())),
            ("pages".to_string(), component_list_nbt(&self.pages)),
        ])
    }

    /// 从 Nbt 承载解析；结构不匹配返回 `None`。
    pub fn from_nbt(tag: &NbtTag) -> Option<WrittenBook> {
        let entries = compound_entries(tag)?;
        let title = entry_component(entries, "title")?;
        let author = entry_string(entries, "author")?.to_string();
        let pages = component_list_from_nbt(entries, "pages")?;
        Some(WrittenBook {
            title,
            author,
            pages,
        })
    }
}

/// `ItemStack` 的书承载 typed getter/setter（经组件 id 52/53）。
impl ItemStack {
    /// 读取可写书；无 writable_book_content(52) 组件或结构不匹配时返回 `None`。
    pub fn writable_book(&self) -> Option<WritableBook> {
        match self.components.get(52) {
            Some(ComponentValue::Nbt(tag)) => WritableBook::from_nbt(tag),
            _ => None,
        }
    }

    /// 设置可写书（写入 writable_book_content(52) 组件）。
    pub fn set_writable_book(&mut self, book: &WritableBook) {
        self.components
            .set_at(52, ComponentValue::Nbt(book.to_nbt()));
    }

    /// 读取已写书；无 written_book_content(53) 组件或结构不匹配时返回 `None`。
    pub fn written_book(&self) -> Option<WrittenBook> {
        match self.components.get(53) {
            Some(ComponentValue::Nbt(tag)) => WrittenBook::from_nbt(tag),
            _ => None,
        }
    }

    /// 设置已写书（写入 written_book_content(53) 组件）。
    pub fn set_written_book(&mut self, book: &WrittenBook) {
        self.components
            .set_at(53, ComponentValue::Nbt(book.to_nbt()));
    }
}

/// 护甲值类型（对齐 Java `item/armor` 语义：防御点数与韧性）。
///
/// 纯值类型，不绑定具体组件；运行时取值可经属性注册表或组件解析。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Armor {
    /// 护甲防御点数（`minecraft:armor` 属性值）。
    pub defense: u32,
    /// 护甲韧性（`minecraft:armor_toughness` 属性值）。
    pub toughness: u32,
}

impl Armor {
    /// 以防御点与韧性构造。
    pub fn new(defense: u32, toughness: u32) -> Self {
        Armor { defense, toughness }
    }

    /// 护甲防御点数。
    pub fn defense(&self) -> u32 {
        self.defense
    }

    /// 护甲韧性。
    pub fn toughness(&self) -> u32 {
        self.toughness
    }
}

/// 武器值类型（对齐 Java `item/weapon` 语义：攻击伤害与攻击速度）。
///
/// 纯值类型，不绑定具体组件；字段对应 `minecraft:attack_damage` /
/// `minecraft:attack_speed` 属性语义。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weapon {
    /// 攻击伤害（基础攻击伤害点数）。
    pub attack_damage: f32,
    /// 攻击速度（每击间隔的倒数语义，越大越快）。
    pub attack_speed: f32,
}

impl Weapon {
    /// 以攻击伤害与攻击速度构造。
    pub fn new(attack_damage: f32, attack_speed: f32) -> Self {
        Weapon {
            attack_damage,
            attack_speed,
        }
    }

    /// 攻击伤害。
    pub fn attack_damage(&self) -> f32 {
        self.attack_damage
    }

    /// 攻击速度。
    pub fn attack_speed(&self) -> f32 {
        self.attack_speed
    }
}

/// 工具值类型（对齐 Java `item/tool` 语义：挖掘速度与挖掘等级）。
///
/// 纯值类型，不绑定具体组件；`mining_level` 对应工具可挖掘的方块硬度等级门槛。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tool {
    /// 挖掘速度倍率（相对徒手的挖掘速度）。
    pub mining_speed: f32,
    /// 挖掘等级（可开采的方块等级）。
    pub mining_level: u32,
}

impl Tool {
    /// 以挖掘速度与挖掘等级构造。
    pub fn new(mining_speed: f32, mining_level: u32) -> Self {
        Tool {
            mining_speed,
            mining_level,
        }
    }

    /// 挖掘速度倍率。
    pub fn mining_speed(&self) -> f32 {
        self.mining_speed
    }

    /// 挖掘等级。
    pub fn mining_level(&self) -> u32 {
        self.mining_level
    }
}

/// 弩值类型（对齐 Java `item/crossbow` 语义：蓄能状态与装填弹丸）。
///
/// 纯值类型，不绑定具体组件；`charged_projectiles` 对应
/// `charged_projectiles`(47) 组件的运行时视图。
#[derive(Clone, Debug, PartialEq)]
pub struct Crossbow {
    /// 是否已蓄能（装填完成）。
    pub charged: bool,
    /// 已装填的弹丸（如箭）。
    pub charged_projectiles: Vec<ItemStack>,
}

impl Crossbow {
    /// 以蓄能状态与装填弹丸构造。
    pub fn new(charged: bool, charged_projectiles: Vec<ItemStack>) -> Self {
        Crossbow {
            charged,
            charged_projectiles,
        }
    }

    /// 是否已蓄能。
    pub fn is_charged(&self) -> bool {
        self.charged
    }

    /// 已装填弹丸（只读切片）。
    pub fn charged_projectiles(&self) -> &[ItemStack] {
        &self.charged_projectiles
    }
}

/// 将组件列表编码为 Nbt `List`（每页一个 Component Compound）。
fn component_list_nbt(pages: &[Component]) -> NbtTag {
    NbtTag::List(pages.iter().map(Component::to_nbt).collect())
}

/// 从 Compound entries 提取指定键为 `List` 的组件列表。
fn component_list_from_nbt(entries: &[(String, NbtTag)], key: &str) -> Option<Vec<Component>> {
    let list = entries.iter().find(|(k, _)| k.as_str() == key)?;
    let items = match &list.1 {
        NbtTag::List(items) => items,
        _ => return None,
    };
    let mut pages = Vec::with_capacity(items.len());
    for item in items {
        pages.push(Component::from_nbt(item).ok()?);
    }
    Some(pages)
}

/// 提取 Compound 的 entries；非 Compound 返回 `None`。
fn compound_entries(tag: &NbtTag) -> Option<&[(String, NbtTag)]> {
    match tag {
        NbtTag::Compound(e) => Some(e.as_slice()),
        _ => None,
    }
}

/// 提取指定键为 Component Compound 的值。
fn entry_component(entries: &[(String, NbtTag)], key: &str) -> Option<Component> {
    let entry = entries.iter().find(|(k, _)| k.as_str() == key)?;
    Component::from_nbt(&entry.1).ok()
}

/// 提取指定键为 String 的值。
fn entry_string<'a>(entries: &'a [(String, NbtTag)], key: &str) -> Option<&'a str> {
    let entry = entries.iter().find(|(k, _)| k.as_str() == key)?;
    match &entry.1 {
        NbtTag::String(s) => Some(s.as_str()),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::protocol::byte_buf::ByteBuffer;
    use crate::protocol::error::ProtocolError;

    /// 测试辅助：整栈线格式 roundtrip，返回解码后的物品（测试内部使用）。
    fn roundtrip(item: &ItemStack) -> ItemStack {
        let mut buf = ByteBuffer::with_capacity(256);
        match crate::item_stack::encode_item_stack(item, &mut buf) {
            Ok(()) => {}
            Err(e) => panic!("encode_item_stack 失败：{e}"),
        }
        let mut buf = ByteBuffer::new(buf.into_inner());
        match crate::item_stack::decode_item_stack(&mut buf) {
            Ok(decoded) => decoded,
            Err(e) => panic!("decode_item_stack 失败：{e}"),
        }
    }

    #[test]
    fn writable_book_roundtrip() {
        let book = WritableBook::new(vec![
            Component::text("第一页"),
            Component::Text {
                text: "第二页".to_string(),
                style: crate::text_component::Style::with_color(0xFF_FF_00_00),
            },
        ]);
        let mut item = ItemStack::new(386, 1); // writable_book
        item.set_writable_book(&book);
        assert_eq!(item.writable_book().unwrap(), book);
        assert_eq!(item.writable_book().unwrap().pages().len(), 2);
        // 未设置时返回 None。
        assert!(ItemStack::new(264, 1).writable_book().is_none());
        // 组件 id=52 不与 custom_data(0)/enchantments(13) 冲突。
        assert!(item.components.get(52).is_some());
        assert!(item.components.get(0).is_none());
    }

    #[test]
    fn writable_book_wire_roundtrip() {
        // 经线格式（组件 patch）编解码无损。
        let book = WritableBook::new(vec![Component::text("页")]);
        let mut item = ItemStack::new(386, 1);
        item.set_writable_book(&book);
        let decoded = roundtrip(&item);
        assert_eq!(decoded, item);
        assert_eq!(decoded.writable_book().unwrap().pages().len(), 1);
    }

    #[test]
    fn written_book_roundtrip() {
        let book = WrittenBook::new(
            Component::text("冒险日志"),
            "Steve".to_string(),
            vec![
                Component::text("第一章"),
                Component::Translatable {
                    key: "book.test".to_string(),
                    fallback: Some("结尾".to_string()),
                    args: vec![Component::text("!")],
                },
            ],
        );
        let mut item = ItemStack::new(387, 1); // written_book
        item.set_written_book(&book);
        assert_eq!(item.written_book().unwrap(), book);
        assert_eq!(item.written_book().unwrap().author(), "Steve");
        assert_eq!(item.written_book().unwrap().pages().len(), 2);
        // 未设置时返回 None。
        assert!(ItemStack::new(264, 1).written_book().is_none());
    }

    #[test]
    fn written_book_wire_roundtrip() {
        let book = WrittenBook::new(
            Component::text("标题"),
            "Alex".to_string(),
            vec![Component::text("正文")],
        );
        let mut item = ItemStack::new(387, 1);
        item.set_written_book(&book);
        let decoded = roundtrip(&item);
        assert_eq!(decoded, item);
        assert_eq!(decoded.written_book().unwrap().title().plain_text(), "标题");
    }

    #[test]
    fn writable_book_rejects_malformed_nbt() {
        // 非 Compound。
        assert!(WritableBook::from_nbt(&NbtTag::Int(1)).is_none());
        // 缺 pages 键。
        assert!(WritableBook::from_nbt(&NbtTag::Compound(vec![])).is_none());
        // pages 不是 List。
        let bad = NbtTag::Compound(vec![("pages".to_string(), NbtTag::Int(0))]);
        assert!(WritableBook::from_nbt(&bad).is_none());
        // 页元素不是合法 Component（text 键类型错误）。
        let bad_page = NbtTag::Compound(vec![(
            "pages".to_string(),
            NbtTag::List(vec![NbtTag::Compound(vec![(
                "text".to_string(),
                NbtTag::Int(7),
            )])]),
        )]);
        assert!(WritableBook::from_nbt(&bad_page).is_none());
    }

    #[test]
    fn written_book_rejects_malformed_nbt() {
        // 缺 author。
        let missing_author = NbtTag::Compound(vec![
            ("title".to_string(), Component::text("t").to_nbt()),
            (
                "pages".to_string(),
                NbtTag::List(vec![Component::text("p").to_nbt()]),
            ),
        ]);
        assert!(WrittenBook::from_nbt(&missing_author).is_none());
        // author 不是 String。
        let bad_author = NbtTag::Compound(vec![
            ("title".to_string(), Component::text("t").to_nbt()),
            ("author".to_string(), NbtTag::Int(0)),
            ("pages".to_string(), NbtTag::List(vec![])),
        ]);
        assert!(WrittenBook::from_nbt(&bad_author).is_none());
    }

    #[test]
    fn armor_construction_and_accessors() {
        let armor = Armor::new(8, 2);
        assert_eq!(armor.defense(), 8);
        assert_eq!(armor.toughness(), 2);
        assert_eq!(armor.defense, 8);
        assert_eq!(armor.toughness, 2);
        // 无护甲：全 0。
        assert_eq!(Armor::new(0, 0).defense(), 0);
    }

    #[test]
    fn weapon_construction_and_accessors() {
        let weapon = Weapon::new(7.0, 1.6);
        assert_eq!(weapon.attack_damage(), 7.0);
        assert_eq!(weapon.attack_speed(), 1.6);
    }

    #[test]
    fn tool_construction_and_accessors() {
        let tool = Tool::new(6.0, 3);
        assert_eq!(tool.mining_speed(), 6.0);
        assert_eq!(tool.mining_level(), 3);
    }

    #[test]
    fn crossbow_construction_and_accessors() {
        let projectiles = vec![ItemStack::new(262, 1)]; // arrow
        let loaded = Crossbow::new(true, projectiles.clone());
        assert!(loaded.is_charged());
        assert_eq!(loaded.charged_projectiles(), projectiles.as_slice());
        assert_eq!(loaded.charged_projectiles.len(), 1);
        let empty = Crossbow::new(false, Vec::new());
        assert!(!empty.is_charged());
        assert!(empty.charged_projectiles().is_empty());
    }

    #[test]
    fn unknown_component_id_still_rejected() {
        // 既有拒绝路径不受 T11 扩展影响：id=5（use_effects）仍返回 Unsupported。
        let mut buf = ByteBuffer::with_capacity(16);
        buf.put_varint(1);
        buf.put_varint(264);
        buf.put_varint(1);
        buf.put_varint(0);
        buf.put_varint(5);
        buf.put_varint(0);
        let mut buf = ByteBuffer::new(buf.into_inner());
        assert_eq!(
            crate::item_stack::decode_item_stack(&mut buf),
            Err(ProtocolError::UnsupportedComponents)
        );
    }
}
