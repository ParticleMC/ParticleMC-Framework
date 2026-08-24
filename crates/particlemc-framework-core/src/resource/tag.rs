//! Tag<T> 通用数据存储框架（R6）：类型化键 + 序列化器 + 处理器。
//!
//! 语义对齐 Java `net.minestom.server.tag`（权威参考 `java/.../server/tag/`，
//! 只对齐语义、不复制翻译 Java）：
//!
//! - [`Tag<T>`]：`&'static str` 键名 + [`PhantomData<T>`] 携带目标类型，
//!   `Tag::string(...)` / `Tag::int(...)` 等工厂构造类型化键，`key()` 取键名；
//! - [`TagSerializer<T>`]：类型 ↔ NBT 双向序列化（`encode` / `decode`），
//!   [`TagHandler`] 以 `HashMap<String, NbtTag>` 存储值；
//! - 类型映射对齐 Java `Serializers`：`String`→`TAG_String`、`i32`→`TAG_Int`、
//!   `f64`→`TAG_Double`、`bool`→`TAG_Byte`(0/1)、`i64`→`TAG_Long`、
//!   `f32`→`TAG_Float`、`Component`→Compound（`to_nbt` / `from_nbt`）、
//!   `ItemStack`→`TAG_ByteArray`（承载 `encode_item_stack` / `decode_item_stack`
//!   网络线格式字节，见 [`TagSerializer<ItemStack>`] 实现文档）；
//! - [`Taggable`] trait 供实体 / 实例挂载标签（`set_tag` / `get_tag` /
//!   `has_tag` / `remove_tag` 默认方法）。
//!
//! 见 `.specs/complete-missing-subsystems/spec.md`（R6）。
//!
//! 变更标识符：`complete-missing-subsystems`。

use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;

use crate::item_stack::{ItemStack, decode_item_stack, encode_item_stack};
use crate::protocol::byte_buf::ByteBuffer;
use crate::protocol::nbt::NbtTag;
use crate::text_component::Component;

/// Tag 序列化错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagError {
    /// 解码失败：NBT 类型匹配但值无法还原（如 Component 结构非法）。
    DecodeFailed,
    /// 类型不匹配：存储的 NBT 类型与目标类型期望不符。
    UnknownType,
}

impl fmt::Display for TagError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TagError::DecodeFailed => write!(f, "Tag 解码失败：NBT 类型匹配但值无法还原"),
            TagError::UnknownType => write!(f, "Tag 类型不匹配：存储的 NBT 类型与目标类型不符"),
        }
    }
}

impl std::error::Error for TagError {}

/// 类型到 NBT 的双向序列化器（对齐 Java `TagSerializer`）。
///
/// `decode` 的错误分支约定：NBT 类型与目标不符返回 [`TagError::UnknownType`]，
/// 类型匹配但值非法返回 [`TagError::DecodeFailed`]。
pub trait TagSerializer<T> {
    /// 将值编码为 NBT tag。
    fn encode(value: &T) -> NbtTag;

    /// 从 NBT tag 解码；失败返回对应 [`TagError`]。
    fn decode(tag: &NbtTag) -> Result<T, TagError>;
}

impl TagSerializer<String> for String {
    fn encode(value: &String) -> NbtTag {
        NbtTag::String(value.clone())
    }

    fn decode(tag: &NbtTag) -> Result<String, TagError> {
        match tag {
            NbtTag::String(s) => Ok(s.clone()),
            _ => Err(TagError::UnknownType),
        }
    }
}

impl TagSerializer<i32> for i32 {
    fn encode(value: &i32) -> NbtTag {
        NbtTag::Int(*value)
    }

    fn decode(tag: &NbtTag) -> Result<i32, TagError> {
        match tag {
            NbtTag::Int(v) => Ok(*v),
            _ => Err(TagError::UnknownType),
        }
    }
}

impl TagSerializer<f64> for f64 {
    fn encode(value: &f64) -> NbtTag {
        NbtTag::Double(*value)
    }

    fn decode(tag: &NbtTag) -> Result<f64, TagError> {
        match tag {
            NbtTag::Double(v) => Ok(*v),
            _ => Err(TagError::UnknownType),
        }
    }
}

impl TagSerializer<bool> for bool {
    /// 以 `TAG_Byte` 承载：`true` → `1`，`false` → `0`（对齐 Java `Serializers.BOOLEAN`）。
    fn encode(value: &bool) -> NbtTag {
        NbtTag::Byte(if *value { 1 } else { 0 })
    }

    fn decode(tag: &NbtTag) -> Result<bool, TagError> {
        match tag {
            NbtTag::Byte(v) => Ok(*v != 0),
            _ => Err(TagError::UnknownType),
        }
    }
}

impl TagSerializer<i64> for i64 {
    fn encode(value: &i64) -> NbtTag {
        NbtTag::Long(*value)
    }

    fn decode(tag: &NbtTag) -> Result<i64, TagError> {
        match tag {
            NbtTag::Long(v) => Ok(*v),
            _ => Err(TagError::UnknownType),
        }
    }
}

impl TagSerializer<f32> for f32 {
    fn encode(value: &f32) -> NbtTag {
        NbtTag::Float(*value)
    }

    fn decode(tag: &NbtTag) -> Result<f32, TagError> {
        match tag {
            NbtTag::Float(v) => Ok(*v),
            _ => Err(TagError::UnknownType),
        }
    }
}

impl TagSerializer<Component> for Component {
    /// 以 Compound 承载（`Component::to_nbt`）。
    fn encode(value: &Component) -> NbtTag {
        value.to_nbt()
    }

    /// 经 `Component::from_nbt` 还原；非 Compound 或结构非法映射为
    /// [`TagError::DecodeFailed`]。
    fn decode(tag: &NbtTag) -> Result<Component, TagError> {
        Component::from_nbt(tag).map_err(|_| TagError::DecodeFailed)
    }
}

impl TagSerializer<ItemStack> for ItemStack {
    /// 以 `TAG_ByteArray` 承载：内部字节为 1.21.11 网络线格式
    /// （[`encode_item_stack`] 输出）。编码失败（材料 id 越界等极端情况）安全
    /// 降级为空 `ByteArray`，解码时得到 [`ItemStack::AIR`]。
    ///
    /// 择一说明：本实现采用 ByteArray 承载而非 NBT Compound——Rust 侧物品
    /// 栈的权威线格式为 `ByteBuffer` 编解码（`item_stack.rs`），未实现 Java
    /// `toItemNBT` 的 Compound 表达，ByteArray 封装复用既有线格式保证 roundtrip。
    fn encode(value: &ItemStack) -> NbtTag {
        let mut buf = ByteBuffer::new(Vec::new());
        match encode_item_stack(value, &mut buf) {
            Ok(()) => NbtTag::ByteArray(buf.into_inner()),
            Err(_) => NbtTag::ByteArray(Vec::new()),
        }
    }

    fn decode(tag: &NbtTag) -> Result<ItemStack, TagError> {
        let bytes = match tag {
            NbtTag::ByteArray(bytes) => bytes,
            _ => return Err(TagError::UnknownType),
        };
        let mut buf = ByteBuffer::new(bytes.clone());
        decode_item_stack(&mut buf).map_err(|_| TagError::DecodeFailed)
    }
}

/// 类型化键：`&'static str` 键名 + [`PhantomData<T>`] 携带目标类型。
///
/// 对齐 Java `Tag<T>` 语义（键名 + 类型化读写），以 Rust 类型系统表达；
/// 构造请用各类型工厂（`Tag::string` / `Tag::int` / ...）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tag<T>(pub &'static str, PhantomData<T>);

impl<T> Tag<T> {
    /// 键名字符串（创建时指定的 key）。
    pub const fn key(&self) -> &'static str {
        self.0
    }
}

impl Tag<String> {
    /// 字符串键（`TAG_String` 承载）。
    pub const fn string(key: &'static str) -> Tag<String> {
        Tag(key, PhantomData)
    }
}

impl Tag<i32> {
    /// 32 位整数键（`TAG_Int` 承载）。
    pub const fn int(key: &'static str) -> Tag<i32> {
        Tag(key, PhantomData)
    }
}

impl Tag<f64> {
    /// 64 位浮点键（`TAG_Double` 承载）。
    pub const fn double(key: &'static str) -> Tag<f64> {
        Tag(key, PhantomData)
    }
}

impl Tag<bool> {
    /// 布尔键（`TAG_Byte` 0/1 承载）。
    pub const fn boolean(key: &'static str) -> Tag<bool> {
        Tag(key, PhantomData)
    }
}

impl Tag<i64> {
    /// 64 位整数键（`TAG_Long` 承载）。
    pub const fn long(key: &'static str) -> Tag<i64> {
        Tag(key, PhantomData)
    }
}

impl Tag<f32> {
    /// 32 位浮点键（`TAG_Float` 承载）。
    pub const fn float(key: &'static str) -> Tag<f32> {
        Tag(key, PhantomData)
    }
}

impl Tag<Component> {
    /// 文本组件键（Compound 承载，经 `Component::to_nbt` / `from_nbt`）。
    pub const fn component(key: &'static str) -> Tag<Component> {
        Tag(key, PhantomData)
    }
}

impl Tag<ItemStack> {
    /// 物品栈键（`TAG_ByteArray` 承载网络线格式）。
    pub const fn item_stack(key: &'static str) -> Tag<ItemStack> {
        Tag(key, PhantomData)
    }
}

/// 通用键值存储：以 `HashMap<String, NbtTag>` 保存任意类型化值。
///
/// 对齐 Java `TagHandler`（`setTag`/`getTag`/`removeTag`/`asCompound`/
/// `fromCompound`）。`get_tag` 解码失败安全降级为 `None`，不向调用方抛出。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TagHandler {
    values: HashMap<String, NbtTag>,
}

impl TagHandler {
    /// 构造空处理器。
    pub fn new() -> Self {
        TagHandler {
            values: HashMap::new(),
        }
    }

    /// 按 `tag` 键写入值（编码为 NBT 存储；同键覆盖）。
    pub fn set_tag<T: TagSerializer<T>>(&mut self, tag: &Tag<T>, value: T) {
        self.values.insert(tag.key().to_owned(), T::encode(&value));
    }

    /// 按 `tag` 键读取并解码；键缺失或解码失败返回 `None`。
    pub fn get_tag<T: TagSerializer<T>>(&self, tag: &Tag<T>) -> Option<T> {
        let stored = self.values.get(tag.key())?;
        T::decode(stored).ok()
    }

    /// 是否存在该键（不校验存储值的类型）。
    pub fn has_tag<T>(&self, tag: &Tag<T>) -> bool {
        self.values.contains_key(tag.key())
    }

    /// 移除键；键存在并移除返回 `true`。
    pub fn remove_tag(&mut self, tag: &Tag<impl Sized>) -> bool {
        self.values.remove(tag.key()).is_some()
    }

    /// 全部键名（字典序，保证确定性输出）。
    pub fn keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.values.keys().cloned().collect();
        keys.sort();
        keys
    }

    /// 序列化为全部键值的 Compound（条目按键名排序，保证确定性）。
    pub fn to_nbt(&self) -> NbtTag {
        let mut entries: Vec<(String, NbtTag)> = self
            .values
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        NbtTag::Compound(entries)
    }

    /// 从 Compound 加载全部条目；非 Compound 输入返回空处理器（安全降级）。
    pub fn from_nbt(tag: &NbtTag) -> Self {
        let mut handler = TagHandler::new();
        if let NbtTag::Compound(entries) = tag {
            for (name, value) in entries {
                handler.values.insert(name.clone(), value.clone());
            }
        }
        handler
    }
}

/// 可打标签对象：暴露 [`TagHandler`] 的读写访问。
///
/// 对齐 Java `Taggable`。实现方只需提供 [`Taggable::tag_handler`] 与
/// [`Taggable::tag_handler_mut`]，即可获得 `set_tag` / `get_tag` / `has_tag` /
/// `remove_tag` 默认方法。
pub trait Taggable {
    /// 只读处理器。
    fn tag_handler(&self) -> &TagHandler;

    /// 可变处理器。
    fn tag_handler_mut(&mut self) -> &mut TagHandler;

    /// 写入标签。
    fn set_tag<T: TagSerializer<T>>(&mut self, tag: &Tag<T>, value: T) {
        self.tag_handler_mut().set_tag(tag, value);
    }

    /// 读取标签（缺失或解码失败返回 `None`）。
    fn get_tag<T: TagSerializer<T>>(&self, tag: &Tag<T>) -> Option<T> {
        self.tag_handler().get_tag(tag)
    }

    /// 是否含该键。
    fn has_tag<T>(&self, tag: &Tag<T>) -> bool {
        self.tag_handler().has_tag(tag)
    }

    /// 移除标签；键存在并移除返回 `true`。
    fn remove_tag(&mut self, tag: &Tag<impl Sized>) -> bool {
        self.tag_handler_mut().remove_tag(tag)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn string_roundtrip() {
        let mut h = TagHandler::new();
        let tag = Tag::string("role");
        assert_eq!(tag.key(), "role");
        h.set_tag(&tag, "guard".to_string());
        assert_eq!(h.get_tag(&tag), Some("guard".to_string()));
        // 底层以 TAG_String 存储
        assert_eq!(
            h.to_nbt(),
            NbtTag::Compound(vec![("role".into(), NbtTag::String("guard".into()))])
        );
    }

    #[test]
    fn numeric_roundtrips() {
        let mut h = TagHandler::new();
        h.set_tag(&Tag::int("i"), 42);
        h.set_tag(&Tag::double("d"), 123.456);
        h.set_tag(&Tag::long("l"), -9_000_000_000_i64);
        h.set_tag(&Tag::float("f"), 2.5_f32);
        h.set_tag(&Tag::boolean("b_true"), true);
        h.set_tag(&Tag::boolean("b_false"), false);
        assert_eq!(h.get_tag(&Tag::int("i")), Some(42));
        assert_eq!(h.get_tag(&Tag::double("d")), Some(123.456));
        assert_eq!(h.get_tag(&Tag::long("l")), Some(-9_000_000_000_i64));
        assert_eq!(h.get_tag(&Tag::float("f")), Some(2.5_f32));
        assert_eq!(h.get_tag(&Tag::boolean("b_true")), Some(true));
        assert_eq!(h.get_tag(&Tag::boolean("b_false")), Some(false));
    }

    #[test]
    fn component_roundtrip() {
        let mut h = TagHandler::new();
        let comp = Component::Translatable {
            key: "chat.type.text".to_string(),
            fallback: Some("fallback".to_string()),
            args: vec![Component::text("Steve")],
        };
        let tag = Tag::component("display");
        h.set_tag(&tag, comp.clone());
        assert_eq!(h.get_tag(&tag), Some(comp));
    }

    #[test]
    fn item_stack_roundtrip() {
        let mut h = TagHandler::new();
        let tag = Tag::item_stack("held");
        let item = ItemStack::new(1, 5); // stone ×5
        h.set_tag(&tag, item.clone());
        assert_eq!(h.get_tag(&tag), Some(item));
        // 空气物品（空 ByteArray）同样 roundtrip
        h.set_tag(&tag, ItemStack::AIR);
        assert_eq!(h.get_tag(&tag), Some(ItemStack::AIR));
    }

    #[test]
    fn unknown_key_returns_none() {
        let h = TagHandler::new();
        assert_eq!(h.get_tag(&Tag::string("missing")), None);
    }

    #[test]
    fn type_mismatch_safe_degradation() {
        let mut h = TagHandler::new();
        h.set_tag(&Tag::int("x"), 7);
        // 存 Int 后用其它类型读 → None（不 panic，安全降级）
        assert_eq!(h.get_tag(&Tag::string("x")), None);
        assert_eq!(h.get_tag(&Tag::double("x")), None);
        assert_eq!(h.get_tag(&Tag::boolean("x")), None);
    }

    #[test]
    fn remove_tag() {
        let mut h = TagHandler::new();
        let tag = Tag::string("k");
        assert!(!h.remove_tag(&tag));
        h.set_tag(&tag, "v".to_string());
        assert!(h.has_tag(&tag));
        assert!(h.remove_tag(&tag));
        assert!(!h.remove_tag(&tag));
        assert_eq!(h.get_tag(&tag), None);
        assert!(!h.has_tag(&tag));
    }

    #[test]
    fn keys_sorted() {
        let mut h = TagHandler::new();
        h.set_tag(&Tag::string("zebra"), "a".to_string());
        h.set_tag(&Tag::string("apple"), "b".to_string());
        assert_eq!(h.keys(), vec!["apple".to_string(), "zebra".to_string()]);
    }

    #[test]
    fn to_nbt_from_nbt_roundtrip() {
        let mut h = TagHandler::new();
        h.set_tag(&Tag::string("s"), "hello".to_string());
        h.set_tag(&Tag::int("i"), -1);
        h.set_tag(&Tag::boolean("b"), true);
        h.set_tag(&Tag::component("c"), Component::text("hi"));
        let nbt = h.to_nbt();
        let loaded = TagHandler::from_nbt(&nbt);
        assert_eq!(loaded.to_nbt(), nbt);
        assert_eq!(loaded.get_tag(&Tag::string("s")), Some("hello".to_string()));
        assert_eq!(loaded.get_tag(&Tag::int("i")), Some(-1));
        assert_eq!(loaded.get_tag(&Tag::boolean("b")), Some(true));
        assert_eq!(
            loaded.get_tag(&Tag::component("c")),
            Some(Component::text("hi"))
        );
    }

    #[test]
    fn from_nbt_ignores_non_compound() {
        let h = TagHandler::from_nbt(&NbtTag::Int(1));
        assert!(h.keys().is_empty());
    }

    #[test]
    fn tag_error_branches() {
        // UnknownType：NBT 类型不匹配
        assert_eq!(
            <String as TagSerializer<String>>::decode(&NbtTag::Int(1)),
            Err(TagError::UnknownType)
        );
        assert_eq!(
            <i32 as TagSerializer<i32>>::decode(&NbtTag::String("x".into())),
            Err(TagError::UnknownType)
        );
        assert_eq!(
            <bool as TagSerializer<bool>>::decode(&NbtTag::String("x".into())),
            Err(TagError::UnknownType)
        );
        assert_eq!(
            <ItemStack as TagSerializer<ItemStack>>::decode(&NbtTag::Int(0)),
            Err(TagError::UnknownType)
        );
        // DecodeFailed：类型匹配但值非法（Component 期望 Compound）
        assert_eq!(
            <Component as TagSerializer<Component>>::decode(&NbtTag::Int(0)),
            Err(TagError::DecodeFailed)
        );
        // 成功分支
        assert_eq!(
            <bool as TagSerializer<bool>>::decode(&NbtTag::Byte(1)),
            Ok(true)
        );
        assert_eq!(<i64 as TagSerializer<i64>>::decode(&NbtTag::Long(5)), Ok(5));
        // 布尔 Byte 非 0 即 true（对齐 Java `value != 0`）
        assert_eq!(
            <bool as TagSerializer<bool>>::decode(&NbtTag::Byte(-3)),
            Ok(true)
        );
    }

    #[test]
    fn boolean_stored_as_byte() {
        let mut h = TagHandler::new();
        h.set_tag(&Tag::boolean("flag"), true);
        assert_eq!(
            h.to_nbt(),
            NbtTag::Compound(vec![("flag".into(), NbtTag::Byte(1))])
        );
    }

    #[test]
    fn taggable_default_methods() {
        struct Labeled {
            handler: TagHandler,
        }
        impl Taggable for Labeled {
            fn tag_handler(&self) -> &TagHandler {
                &self.handler
            }
            fn tag_handler_mut(&mut self) -> &mut TagHandler {
                &mut self.handler
            }
        }
        let mut e = Labeled {
            handler: TagHandler::new(),
        };
        e.set_tag(&Tag::string("role"), "guard".to_string());
        assert_eq!(e.get_tag(&Tag::string("role")), Some("guard".to_string()));
        assert!(e.has_tag(&Tag::string("role")));
        assert!(e.remove_tag(&Tag::string("role")));
        assert_eq!(e.get_tag(&Tag::string("role")), None);
    }
}
