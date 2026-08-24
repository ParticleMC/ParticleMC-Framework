//! 物品栈模型与线格式编解码（Minecraft 1.21.11）。
//!
//! 本模块定义 1.21.11 网络线格式下的物品栈值类型 [`ItemStack`] 及其与
//! [`ByteBuffer`] 之间的 [`encode_item_stack`] / [`decode_item_stack`] 编解码。
//!
//! 数据组件容器 [`ItemComponents`] 实现为真实的 `DataComponentPatch`：先写出所有
//! Set 条目，再写出所有 Remove 条目，均按 vanilla 注册表序位（组件 id）排序。
//! 组件值覆盖「C 档」：既有 6 种自定界类型 + 简单自定界全量
//! （[`ComponentValue::Byte`]/[`ComponentValue::Short`]/[`ComponentValue::Long`]/
//! [`ComponentValue::Float`]/[`ComponentValue::Double`]/[`ComponentValue::String`]/
//! [`ComponentValue::Bool`]）+ NBT 承载（[`ComponentValue::Nbt`]，custom_data/
//! enchantments）+ 文本承载（[`ComponentValue::Text`] custom_name /
//! [`ComponentValue::TextList`] lore，经 [`crate::text_component::Component`]↔NBT）。
//!
//! 组件 id 以 Java Minestom 1.21.11 `DataComponents.java` 登记序位（0 基，
//! `DataComponentImpl.register` 用 `NAMESPACES.size()` 赋 id）为权威，逐一核实：
//!
//! - `0` `custom_data` → [`ComponentValue::Nbt`]（NBT Compound）
//! - `1` `max_stack_size` / `2` `max_damage` / `3` `damage`：VarInt → u32
//! - `4` `unbreakable`：Unit（0 字节 marker）
//! - `6` `custom_name` → [`ComponentValue::Text`]（Component↔NBT）
//! - `7` `minimum_attack_charge`：大端 f32（4 字节）
//! - `10` `item_model` → [`ComponentValue::String`]（VarInt 长度 + UTF-8）
//! - `11` `lore` → [`ComponentValue::TextList`]（VarInt 计数 + 各 Component）
//! - `12` `rarity`：VarInt ordinal（0 COMMON / 1 UNCOMMON / 2 RARE / 3 EPIC）
//! - `13` `enchantments` → [`ComponentValue::Nbt`]（以既有 NBT 承载，框架约定；
//!   其 vanilla 网络格式实为 `EnchantmentList` map，T1 边界见 [`decode_value`]）
//! - `21` `enchantment_glint_override` → [`ComponentValue::Bool`]
//! - `50` `potion_duration_scale` → [`ComponentValue::Float`]
//!
//! `Byte/Short/Long/Double` 在 `DataComponents.java` 中无对应顶层网络类型，作为
//! 「简单自定界全量」的通用载体映射到未同步（network type 为 null）的登记位
//! `22`(intangible_projectile)/`45`(map_decorations)/`76`(lock)/`77`(container_loot)，
//! 真实客户端不会发送这些 id，roundtrip 自洽。
//!
//! T11（物品行为 API）扩展：`writable_book_content`(52) / `written_book_content`(53)
//! 亦以 Nbt 承载（书页/标题经 [`crate::text_component::Component`]↔NBT），与
//! enchantments(13) 同一模式。这类「id ≠ 0 的 Nbt 承载」组件经
//! [`ItemComponents::set_at`] 写入，避免 [`ItemComponents::set`] 按
//! [`component_id`] 归一为 custom_data(0)。
//!
//! 未知组件 id 解码返回 [`ProtocolError::UnsupportedComponents`]（安全降级）。
//! 线格式权威来源：Java Minestom 1.21.11 `ItemStackImpl.networkType` 与
//! `DataComponentMapImpl.NetworkTypeImpl`（`write`/`read`）；NBT/COMPONENT 值为
//! `0x0a` 前导 + anonymous Compound payload（`writeNameless`，见 `NbtType`）。
//! 空 patch 编码为 `added = 0`、`removed = 0`，即字节 `00 00`。
//!
//! 变更标识符：`complete-partial-framework-capabilities`（R3 物品组件 C 档）、
//! `complete-missing-subsystems`（R11 物品行为 API：enchantments 与书承载扩展）。

use crate::protocol::byte_buf::ByteBuffer;
use crate::protocol::error::ProtocolError;
use crate::protocol::nbt::{self, NbtError, NbtTag};
use crate::text_component::Component;

/// 单槽堆叠缺省上限（钻石等无 `max_stack_size` 组件时采用）。
///
/// 见 `.specs/complete-partial-framework-capabilities/spec.md`（R3）。
pub const MAX_STACK: u8 = 64;

/// 物品栈（不可变值类型）。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ItemStack {
    /// 协议物品 id（来自 ItemRegistry）。
    pub material: u32,
    /// 数量；0 表示空气。
    pub amount: u8,
    /// 数据组件容器（1.21.11 DataComponentPatch）。
    pub components: ItemComponents,
}

/// 组件 patch 中的单条条目，保持线格式顺序（先全部 Set，再全部 Remove）。
///
/// Set 条目记录线格式上的组件 id（解码自对端或由 [`component_id`] 在
/// [`ItemComponents::set`] 时写入），保证「同值不同 id」（如 custom_data(0) 与
/// enchantments(13) 均为 Nbt）解码后可无损重编码。
#[derive(Clone, Debug, PartialEq)]
pub enum ComponentEntry {
    /// 设置一个组件值（`id` 为线格式组件 id，`value` 为承载值）。
    Set { id: u32, value: ComponentValue },
    /// 移除一个组件（patch 的 removed 部分，仅 id）。
    Remove(u32),
}

/// 1.21.11 数据组件值（C 档：既有 6 种 + 简单自定界全量 + NBT/文本承载）。
#[derive(Clone, Debug, PartialEq)]
pub enum ComponentValue {
    MaxStackSize(u32),
    MaxDamage(u32),
    Damage(u32),
    /// 不可破坏标记（线格式为 0 字节 Unit）。
    Unbreakable,
    MinimumAttackCharge(f32),
    Rarity(ItemRarity),
    /// 有符号字节（1 字节）。
    Byte(i8),
    /// 有符号短整型（2 字节大端）。
    Short(i16),
    /// 有符号长整型（8 字节大端）。
    Long(i64),
    /// 单精度浮点（4 字节大端）。
    Float(f32),
    /// 双精度浮点（8 字节大端）。
    Double(f64),
    /// 字符串（VarInt 字节长度 + UTF-8）。
    String(String),
    /// 布尔（1 字节，0/1）。
    Bool(bool),
    /// NBT Compound（`0x0a` 前导 + anonymous payload），承载 custom_data(0) /
    /// enchantments(13)。
    Nbt(NbtTag),
    /// 文本组件（Component↔NBT），承载 custom_name(6)。
    Text(Component),
    /// 文本组件列表（VarInt 计数 + 各 Component），承载 lore(11)。
    TextList(Vec<Component>),
}

/// 物品稀有度（线格式为 VarInt ordinal）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemRarity {
    Common = 0,
    Uncommon = 1,
    Rare = 2,
    Epic = 3,
}

/// 数据组件容器（1.21.11 DataComponentPatch）。C 档覆盖简单自定界全量与
/// NBT/文本承载组件，未知组件 id 解码返回 UnsupportedComponents。
/// 见 `.specs/complete-partial-framework-capabilities/spec.md`（R3）。
#[derive(Clone, Debug, PartialEq)]
pub struct ItemComponents {
    entries: Vec<ComponentEntry>,
}

impl ItemComponents {
    /// 空组件 patch 常量（线格式 `00 00`）。
    pub const EMPTY: ItemComponents = ItemComponents {
        entries: Vec::new(),
    };

    /// 构造空容器。
    pub fn new() -> Self {
        ItemComponents {
            entries: Vec::new(),
        }
    }

    /// 是否空（无 Set 且无论 Remove）。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 按组件 id（线格式序位）读取第一个匹配的 Set 值（用于 typed getter）。
    pub fn get(&self, id: u32) -> Option<&ComponentValue> {
        for e in &self.entries {
            if let ComponentEntry::Set { id: stored, value } = e
                && *stored == id
            {
                return Some(value);
            }
        }
        None
    }

    /// 设置/替换某组件（按值对应的规范 id 去重，保持顺序）。
    pub fn set(&mut self, value: ComponentValue) {
        let id = component_id(&value);
        self.set_at(id, value);
    }

    /// 按指定线格式组件 id 设置/替换组件值（`[`set`](Self::set)` 的显式 id 变体）。
    ///
    /// 用于 Nbt 承载组件中 id ≠ custom_data(0) 的类型——enchantments(13)、
    /// writable_book_content(52)、written_book_content(53)——避免 `set` 按
    /// [`component_id`] 归一为 custom_data(0) 而与 custom_data 互相覆盖。
    pub fn set_at(&mut self, id: u32, value: ComponentValue) {
        for e in self.entries.iter_mut() {
            if let ComponentEntry::Set {
                id: stored,
                value: v,
            } = e
                && *stored == id
            {
                *v = value;
                return;
            }
        }
        self.entries.push(ComponentEntry::Set { id, value });
    }

    /// 移除某组件（patch 语义：记为 Remove 条目）。
    pub fn remove(&mut self, id: u32) {
        // 先去掉已有 Set（按线格式 id），再确保有一个 Remove（去重）。
        self.entries
            .retain(|e| !matches!(e, ComponentEntry::Set { id: stored, .. } if *stored == id));
        if !self
            .entries
            .iter()
            .any(|e| matches!(e, ComponentEntry::Remove(r) if *r == id))
        {
            self.entries.push(ComponentEntry::Remove(id));
        }
    }

    /// 按组件 id 读取 max_stack_size（typed getter）。
    pub fn max_stack_size(&self) -> Option<u32> {
        match self.get(1) {
            Some(ComponentValue::MaxStackSize(v)) => Some(*v),
            _ => None,
        }
    }

    /// 按组件 id 读取 max_damage（typed getter）。
    pub fn max_damage(&self) -> Option<u32> {
        match self.get(2) {
            Some(ComponentValue::MaxDamage(v)) => Some(*v),
            _ => None,
        }
    }

    /// 按组件 id 读取 damage（typed getter）。
    pub fn damage(&self) -> Option<u32> {
        match self.get(3) {
            Some(ComponentValue::Damage(v)) => Some(*v),
            _ => None,
        }
    }

    /// 是否 unbreakable（typed getter）。
    pub fn is_unbreakable(&self) -> bool {
        self.get(4)
            .is_some_and(|v| matches!(v, ComponentValue::Unbreakable))
    }

    /// 按组件 id 读取 minimum_attack_charge（typed getter）。
    pub fn minimum_attack_charge(&self) -> Option<f32> {
        match self.get(7) {
            Some(ComponentValue::MinimumAttackCharge(v)) => Some(*v),
            _ => None,
        }
    }

    /// 按组件 id 读取 rarity（typed getter）。
    pub fn rarity(&self) -> Option<ItemRarity> {
        match self.get(12) {
            Some(ComponentValue::Rarity(r)) => Some(*r),
            _ => None,
        }
    }

    /// 读取 custom_name(6) 文本组件（typed getter）。
    pub fn custom_name(&self) -> Option<&Component> {
        match self.get(6) {
            Some(ComponentValue::Text(c)) => Some(c),
            _ => None,
        }
    }

    /// 读取 lore(11) 文本组件列表（typed getter）。
    pub fn lore(&self) -> Option<&[Component]> {
        match self.get(11) {
            Some(ComponentValue::TextList(l)) => Some(l.as_slice()),
            _ => None,
        }
    }

    /// 读取 custom_data(0) NBT（typed getter）。
    pub fn custom_data(&self) -> Option<&NbtTag> {
        match self.get(0) {
            Some(ComponentValue::Nbt(t)) => Some(t),
            _ => None,
        }
    }

    /// 读取 enchantments(13) NBT（typed getter）。
    ///
    /// 注意：T1 以既有 NBT 承载 enchantments（框架约定），其 vanilla 网络格式
    /// 实为 `EnchantmentList` map，真实客户端互操作留待 T5/T6 细化。
    pub fn enchantments(&self) -> Option<&NbtTag> {
        match self.get(13) {
            Some(ComponentValue::Nbt(t)) => Some(t),
            _ => None,
        }
    }
}

impl Default for ItemComponents {
    fn default() -> Self {
        Self::new()
    }
}

impl ItemStack {
    /// 空气物品常量（amount == 0）。
    pub const AIR: ItemStack = ItemStack {
        material: 0,
        amount: 0,
        components: ItemComponents::EMPTY,
    };

    /// 是否为空气（amount == 0）。
    pub fn is_air(&self) -> bool {
        self.amount == 0
    }

    /// 以 material 与 amount 构造；amount == 0 时语义等同 AIR（直接返回 [`AIR`](ItemStack::AIR)）。
    pub fn new(material: u32, amount: u8) -> Self {
        if amount == 0 {
            return ItemStack::AIR;
        }
        ItemStack {
            material,
            amount,
            components: ItemComponents::EMPTY,
        }
    }

    /// 该物品的单槽堆叠上限：读 max_stack_size 组件，缺省 64。
    /// 见 `.specs/complete-partial-framework-capabilities/spec.md`（R3）。
    pub fn max_stack(&self) -> u8 {
        self.components
            .max_stack_size()
            .and_then(|v| u8::try_from(v).ok())
            .unwrap_or(MAX_STACK)
    }
}

/// 组件规范 id（vanilla 注册表序位，0 基，逐一核对 `DataComponents.java`）。
///
/// 见模块文档的 id 映射表。`Nbt` 的规范 id 为 custom_data(0)（enchantments(13)
/// 解码后按线格式 id 存储，重编码不丢失）；`Byte/Short/Long/Double` 映射到
/// 未同步登记位（无真实客户端互操作语义，roundtrip 自洽）。
fn component_id(v: &ComponentValue) -> u32 {
    match v {
        ComponentValue::MaxStackSize(_) => 1,
        ComponentValue::MaxDamage(_) => 2,
        ComponentValue::Damage(_) => 3,
        ComponentValue::Unbreakable => 4,
        ComponentValue::MinimumAttackCharge(_) => 7,
        ComponentValue::Rarity(_) => 12,
        ComponentValue::Byte(_) => 22, // intangible_projectile（未同步）
        ComponentValue::Short(_) => 45, // map_decorations（未同步）
        ComponentValue::Long(_) => 76, // lock（未同步）
        ComponentValue::Float(_) => 50, // potion_duration_scale（FLOAT）
        ComponentValue::Double(_) => 77, // container_loot（未同步）
        ComponentValue::String(_) => 10, // item_model（STRING）
        ComponentValue::Bool(_) => 21, // enchantment_glint_override（BOOLEAN）
        ComponentValue::Nbt(_) => 0,   // custom_data（NBT_COMPOUND）
        ComponentValue::Text(_) => 6,  // custom_name（COMPONENT）
        ComponentValue::TextList(_) => 11, // lore（COMPONENT.list）
    }
}

/// 写出一个带 `0x0a` 前导的 anonymous NBT Compound（对齐 Java `NbtType.writeNameless`）。
fn write_nbt_compound(buf: &mut ByteBuffer, tag: &NbtTag) -> Result<(), ProtocolError> {
    // encode_anonymous 仅接受 Compound，非 Compound 返回 InvalidListType → InvalidValue。
    let bytes = nbt::encode_anonymous(tag).map_err(|_| ProtocolError::InvalidValue)?;
    buf.put_u8(0x0a);
    buf.put_bytes(&bytes);
    Ok(())
}

/// 读取一个带 `0x0a` 前导的 anonymous NBT Compound 并推进游标。
fn read_nbt_compound(buf: &mut ByteBuffer) -> Result<NbtTag, ProtocolError> {
    let tag_id = buf.get_u8()?;
    if tag_id != 0x0a {
        return Err(ProtocolError::InvalidValue);
    }
    let rest = buf
        .as_slice()
        .get(buf.position()..)
        .ok_or(ProtocolError::UnexpectedEof)?;
    let (tag, consumed) = nbt::decode_anonymous(rest).map_err(|e| match e {
        NbtError::UnexpectedEof => ProtocolError::UnexpectedEof,
        _ => ProtocolError::InvalidValue,
    })?;
    buf.get_bytes(consumed)?;
    Ok(tag)
}

/// 按组件 id 写出值（无长度前缀，trusted 模式）。
fn encode_value(id: u32, v: &ComponentValue, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
    match (id, v) {
        (1, ComponentValue::MaxStackSize(n))
        | (2, ComponentValue::MaxDamage(n))
        | (3, ComponentValue::Damage(n)) => {
            let n = i32::try_from(*n).map_err(|_| ProtocolError::InvalidValue)?;
            buf.put_varint(n);
        }
        (4, ComponentValue::Unbreakable) => { /* Unit：0 字节，不写 */ }
        (7, ComponentValue::MinimumAttackCharge(f)) => {
            buf.put_f32(*f);
        }
        (12, ComponentValue::Rarity(r)) => {
            // ordinal 为 0-3，显式匹配避免 `as` 缩窄转换告警。
            let ord = match r {
                ItemRarity::Common => 0u32,
                ItemRarity::Uncommon => 1,
                ItemRarity::Rare => 2,
                ItemRarity::Epic => 3,
            };
            let ord = i32::try_from(ord).map_err(|_| ProtocolError::InvalidValue)?;
            buf.put_varint(ord);
        }
        (22, ComponentValue::Byte(b)) => buf.put_i8(*b),
        (45, ComponentValue::Short(s)) => buf.put_i16(*s),
        (76, ComponentValue::Long(l)) => buf.put_i64(*l),
        (50, ComponentValue::Float(f)) => buf.put_f32(*f),
        (77, ComponentValue::Double(d)) => buf.put_f64(*d),
        (10, ComponentValue::String(s)) => buf.put_string(s),
        (21, ComponentValue::Bool(b)) => buf.put_bool(*b),
        (0, ComponentValue::Nbt(tag))
        | (13, ComponentValue::Nbt(tag))
        | (52, ComponentValue::Nbt(tag))
        | (53, ComponentValue::Nbt(tag)) => {
            write_nbt_compound(buf, tag)?;
        }
        (6, ComponentValue::Text(c)) => {
            write_nbt_compound(buf, &c.to_nbt())?;
        }
        (11, ComponentValue::TextList(list)) => {
            let len = i32::try_from(list.len()).map_err(|_| ProtocolError::InvalidValue)?;
            buf.put_varint(len);
            for c in list {
                write_nbt_compound(buf, &c.to_nbt())?;
            }
        }
        _ => return Err(ProtocolError::InvalidValue),
    }
    Ok(())
}

/// 按组件 id 读取值（无长度前缀，trusted 模式）。未知 id → UnsupportedComponents。
fn decode_value(id: u32, buf: &mut ByteBuffer) -> Result<ComponentValue, ProtocolError> {
    match id {
        1 => Ok(ComponentValue::MaxStackSize(
            u32::try_from(buf.get_varint()?).map_err(|_| ProtocolError::InvalidValue)?,
        )),
        2 => Ok(ComponentValue::MaxDamage(
            u32::try_from(buf.get_varint()?).map_err(|_| ProtocolError::InvalidValue)?,
        )),
        3 => Ok(ComponentValue::Damage(
            u32::try_from(buf.get_varint()?).map_err(|_| ProtocolError::InvalidValue)?,
        )),
        4 => Ok(ComponentValue::Unbreakable), // 读 0 字节
        7 => Ok(ComponentValue::MinimumAttackCharge(buf.get_f32()?)),
        12 => {
            let ord = buf.get_varint()?;
            let r = match ord {
                0 => ItemRarity::Common,
                1 => ItemRarity::Uncommon,
                2 => ItemRarity::Rare,
                3 => ItemRarity::Epic,
                _ => return Err(ProtocolError::InvalidValue),
            };
            Ok(ComponentValue::Rarity(r))
        }
        0 => Ok(ComponentValue::Nbt(read_nbt_compound(buf)?)),
        6 => {
            let tag = read_nbt_compound(buf)?;
            let c = Component::from_nbt(&tag).map_err(|_| ProtocolError::InvalidValue)?;
            Ok(ComponentValue::Text(c))
        }
        13 => Ok(ComponentValue::Nbt(read_nbt_compound(buf)?)),
        // T11：书承载（writable_book_content / written_book_content）以 Nbt 承载，
        // 页/标题文本经 Component↔NBT 编解码，与 enchantments(13) 同模式。
        52 | 53 => Ok(ComponentValue::Nbt(read_nbt_compound(buf)?)),
        10 => Ok(ComponentValue::String(buf.get_string()?)),
        11 => {
            let count = buf.get_varint()?;
            let count_usize = usize::try_from(count).map_err(|_| ProtocolError::InvalidValue)?;
            if count_usize > 256 {
                return Err(ProtocolError::InvalidValue); // lore：COMPONENT.list(256)
            }
            let mut list = Vec::with_capacity(count_usize);
            for _ in 0..count_usize {
                let tag = read_nbt_compound(buf)?;
                list.push(Component::from_nbt(&tag).map_err(|_| ProtocolError::InvalidValue)?);
            }
            Ok(ComponentValue::TextList(list))
        }
        21 => Ok(ComponentValue::Bool(buf.get_bool()?)),
        22 => Ok(ComponentValue::Byte(buf.get_i8()?)),
        45 => Ok(ComponentValue::Short(buf.get_i16()?)),
        50 => Ok(ComponentValue::Float(buf.get_f32()?)),
        76 => Ok(ComponentValue::Long(buf.get_i64()?)),
        77 => Ok(ComponentValue::Double(buf.get_f64()?)),
        _ => Err(ProtocolError::UnsupportedComponents),
    }
}

/// 按 1.21.11 线格式写出物品栈（air ⇒ 仅 VarInt 0）。
///
/// 编码顺序（非空物品）：
/// 1. `count` VarInt
/// 2. `material` VarInt
/// 3. 组件 patch（DataComponentPatch，patch 模式）：
///    `added` VarInt、`removed` VarInt、依次所有 Set 条目 `(id, 内联值)`、
///    再依次所有 Remove 条目 `id`（均 trusted 模式，无长度前缀）。
pub fn encode_item_stack(item: &ItemStack, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
    if item.is_air() {
        buf.put_varint(0);
        return Ok(());
    }
    // 1. count（VarInt）。u8 → i32 为拓宽转换，使用 From 安全完成。
    let count = i32::from(item.amount);
    buf.put_varint(count);
    // 2. material（VarInt）。u32 → i32 可能为缩窄（物品 id 越界时失败）。
    let material = i32::try_from(item.material).map_err(|_| ProtocolError::InvalidValue)?;
    buf.put_varint(material);
    // 3. DataComponentPatch（patch 模式）：added, removed, [Set...], [Remove...]
    let added: i32 = item
        .components
        .entries
        .iter()
        .filter(|e| matches!(e, ComponentEntry::Set { .. }))
        .count() as i32;
    let removed: i32 = item
        .components
        .entries
        .iter()
        .filter(|e| matches!(e, ComponentEntry::Remove(_)))
        .count() as i32;
    buf.put_varint(added);
    buf.put_varint(removed);
    for e in &item.components.entries {
        if let ComponentEntry::Set { id, value } = e {
            let id_i32 = i32::try_from(*id).map_err(|_| ProtocolError::InvalidValue)?;
            buf.put_varint(id_i32);
            encode_value(*id, value, buf)?;
        }
    }
    for e in &item.components.entries {
        if let ComponentEntry::Remove(id) = e {
            let id = i32::try_from(*id).map_err(|_| ProtocolError::InvalidValue)?;
            buf.put_varint(id);
        }
    }
    Ok(())
}

/// 按 1.21.11 线格式读入物品栈。
///
/// 解码规则：
/// - 读 `count` VarInt；若 `count <= 0` 返回 [`ItemStack::AIR`]。
/// - 读 `material` VarInt 并安全转为 `u32`。
/// - 读组件 patch：`added` 与 `removed` VarInt；断言 `added + removed ≤ 256`。
/// - 读 `added` 个 `(id, 内联值)`；未知 id 返回 [`ProtocolError::UnsupportedComponents`]。
/// - 读 `removed` 个 `id`，记为 Remove 条目。
pub fn decode_item_stack(buf: &mut ByteBuffer) -> Result<ItemStack, ProtocolError> {
    let count = buf.get_varint()?;
    if count <= 0 {
        return Ok(ItemStack::AIR);
    }
    let material = buf.get_varint()?;
    // i32 → u32：负物品 id 或越界一律视为非法协议值。
    let material = u32::try_from(material).map_err(|_| ProtocolError::InvalidValue)?;
    let added = buf.get_varint()?;
    let removed = buf.get_varint()?;
    if added < 0 || removed < 0 || added + removed > 256 {
        return Err(ProtocolError::InvalidValue);
    }
    let mut comps = ItemComponents::new();
    for _ in 0..added {
        let id = u32::try_from(buf.get_varint()?).map_err(|_| ProtocolError::InvalidValue)?;
        let value = decode_value(id, buf)?; // 未知 id → UnsupportedComponents
        comps.entries.push(ComponentEntry::Set { id, value });
    }
    for _ in 0..removed {
        let id = u32::try_from(buf.get_varint()?).map_err(|_| ProtocolError::InvalidValue)?;
        comps.entries.push(ComponentEntry::Remove(id));
    }
    // count（> 0）转为 u8 可能越界（> 255），需显式处理。
    let amount = u8::try_from(count).map_err(|_| ProtocolError::InvalidValue)?;
    Ok(ItemStack {
        material,
        amount,
        components: comps,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// 测试辅助：成功解码并取值；失败则 panic（仅测试内部使用）。
    fn decode_ok(buf: &mut ByteBuffer) -> ItemStack {
        match decode_item_stack(buf) {
            Ok(v) => v,
            Err(e) => panic!("decode_item_stack 预期成功，实际失败：{e}"),
        }
    }

    /// 测试辅助：成功编码；失败则 panic（仅测试内部使用）。
    fn encode_ok(item: &ItemStack, buf: &mut ByteBuffer) {
        match encode_item_stack(item, buf) {
            Ok(()) => {}
            Err(e) => panic!("encode_item_stack 预期成功，实际失败：{e}"),
        }
    }

    // ── 保留的原有测试（v1 语义下仍应全绿） ──

    #[test]
    fn air_encode_single_zero_byte() {
        let mut b = ByteBuffer::with_capacity(1);
        encode_ok(&ItemStack::AIR, &mut b);
        assert_eq!(b.as_slice(), &[0x00]);
    }

    #[test]
    fn air_via_new_is_air() {
        let air = ItemStack::new(264, 0);
        assert!(air.is_air());
        assert_eq!(air, ItemStack::AIR);
    }

    #[test]
    fn diamond_encode_exact_bytes() {
        // material=264, amount=1, 空组件
        let item = ItemStack::new(264, 1);
        let mut b = ByteBuffer::with_capacity(8);
        encode_ok(&item, &mut b);
        assert_eq!(b.as_slice(), &[0x01, 0x88, 0x02, 0x00, 0x00]);
    }

    #[test]
    fn air_decode_from_zero_varint() {
        let mut b = ByteBuffer::new(vec![0x00]);
        let item = decode_ok(&mut b);
        assert!(item.is_air());
        assert_eq!(item, ItemStack::AIR);
    }

    #[test]
    fn negative_count_decodes_to_air() {
        // VarInt -1 完整编码为五字节；count <= 0 应返回 AIR。
        let mut b = ByteBuffer::with_capacity(8);
        b.put_varint(-1);
        let mut b = ByteBuffer::new(b.into_inner());
        let item = decode_ok(&mut b);
        assert!(item.is_air());
    }

    #[test]
    fn diamond_roundtrip() {
        let item = ItemStack::new(264, 1);
        let mut b = ByteBuffer::with_capacity(8);
        encode_ok(&item, &mut b);
        let mut b = ByteBuffer::new(b.into_inner());
        let decoded = decode_ok(&mut b);
        assert_eq!(decoded, item);
        assert_eq!(decoded.material, 264);
        assert_eq!(decoded.amount, 1);
    }

    #[test]
    fn stackable_amount_roundtrip() {
        let item = ItemStack::new(1, 64); // 64 个石头
        let mut b = ByteBuffer::with_capacity(8);
        encode_ok(&item, &mut b);
        let mut b = ByteBuffer::new(b.into_inner());
        let decoded = decode_ok(&mut b);
        assert_eq!(decoded, item);
        assert_eq!(decoded.amount, 64);
    }

    #[test]
    fn truncated_patch_returns_eof() {
        // 仅写入 count 与 material，省略组件 patch
        let mut b = ByteBuffer::new(Vec::new());
        b.put_varint(1);
        b.put_varint(264);
        let mut b = ByteBuffer::new(b.into_inner());
        assert_eq!(decode_item_stack(&mut b), Err(ProtocolError::UnexpectedEof));
    }

    #[test]
    fn non_empty_patch_rejected() {
        // count=1, material=264, added=1, removed=0, id=5(use_effects，未支持)
        // → 未知 Set 组件 id 返回 UnsupportedComponents。
        let mut b = ByteBuffer::new(Vec::new());
        b.put_varint(1);
        b.put_varint(264);
        b.put_varint(1);
        b.put_varint(0);
        b.put_varint(5); // 任意 bytes 不会读到，因为 id 即触发拒识
        b.put_varint(0);
        let mut b = ByteBuffer::new(b.into_inner());
        assert_eq!(
            decode_item_stack(&mut b),
            Err(ProtocolError::UnsupportedComponents)
        );
    }

    #[test]
    fn removed_only_patch_roundtrip() {
        // v1 接受纯 Remove 条目（去除某组件），未知 id 也合法（仅 marker）。
        // count=1, material=264, added=0, removed=1, id=6
        let mut b = ByteBuffer::new(Vec::new());
        b.put_varint(1);
        b.put_varint(264);
        b.put_varint(0);
        b.put_varint(1);
        b.put_varint(6);
        let mut b = ByteBuffer::new(b.into_inner());
        let item = decode_ok(&mut b);
        assert_eq!(item.amount, 1);
        assert!(!item.components.is_empty());
        assert_eq!(item.components.entries, vec![ComponentEntry::Remove(6)]);
    }

    #[test]
    fn amount_overflow_rejected() {
        // count=256 超出 u8 范围
        let mut b = ByteBuffer::new(Vec::new());
        b.put_varint(256);
        b.put_varint(264);
        b.put_varint(0);
        b.put_varint(0);
        let mut b = ByteBuffer::new(b.into_inner());
        assert_eq!(decode_item_stack(&mut b), Err(ProtocolError::InvalidValue));
    }

    #[test]
    fn negative_material_rejected() {
        // material = -1（VarInt 0xFF 0x01），u32 转换失败
        let mut b = ByteBuffer::new(Vec::new());
        b.put_varint(1);
        b.put_varint(-1);
        b.put_varint(0);
        b.put_varint(0);
        let mut b = ByteBuffer::new(b.into_inner());
        assert_eq!(decode_item_stack(&mut b), Err(ProtocolError::InvalidValue));
    }

    // ── 新增：组件 patch 编解码与错误分支 ──

    #[test]
    fn empty_components_encode_is_00_00() {
        // new(264,1) 默认 EMPTY → 字节 `01 88 02 00 00`。
        let item = ItemStack::new(264, 1);
        assert!(item.components.is_empty());
        let mut b = ByteBuffer::with_capacity(8);
        encode_ok(&item, &mut b);
        assert_eq!(b.as_slice(), &[0x01, 0x88, 0x02, 0x00, 0x00]);
    }

    #[test]
    fn max_stack_size_roundtrip() {
        let mut item = ItemStack::new(264, 1);
        item.components.set(ComponentValue::MaxStackSize(16));
        let mut b = ByteBuffer::with_capacity(8);
        encode_ok(&item, &mut b);
        let mut b = ByteBuffer::new(b.into_inner());
        let decoded = decode_ok(&mut b);
        assert_eq!(decoded.components.max_stack_size(), Some(16));
        assert_eq!(decoded, item);
    }

    #[test]
    fn damage_and_remove_roundtrip() {
        let mut item = ItemStack::new(264, 1);
        item.components.set(ComponentValue::Damage(5));
        item.components.remove(4); // 移除 unbreakable
        let mut b = ByteBuffer::with_capacity(8);
        encode_ok(&item, &mut b);
        let mut b = ByteBuffer::new(b.into_inner());
        let decoded = decode_ok(&mut b);
        assert_eq!(decoded.components.damage(), Some(5));
        assert!(!decoded.components.is_unbreakable());
        // 顺序：先 Set(Damage)，后 Remove(4)。
        assert_eq!(
            decoded.components.entries,
            vec![
                ComponentEntry::Set {
                    id: 3,
                    value: ComponentValue::Damage(5)
                },
                ComponentEntry::Remove(4),
            ]
        );
    }

    #[test]
    fn unbreakable_zero_bytes() {
        let mut item = ItemStack::new(264, 1);
        item.components.set(ComponentValue::Unbreakable);
        let mut b = ByteBuffer::with_capacity(8);
        encode_ok(&item, &mut b);
        // id=4 之后无额外字节（Unit）。解码应正常。
        let mut b = ByteBuffer::new(b.into_inner());
        let decoded = decode_ok(&mut b);
        assert!(decoded.components.is_unbreakable());
    }

    #[test]
    fn rarity_ordinal_roundtrip() {
        let mut item = ItemStack::new(264, 1);
        item.components
            .set(ComponentValue::Rarity(ItemRarity::Rare));
        let mut b = ByteBuffer::with_capacity(8);
        encode_ok(&item, &mut b);
        // 编码应写出 id=12 后 VarInt ordinal 2。
        let encoded = b.as_slice().to_vec();
        assert_eq!(encoded[encoded.len() - 2], 12); // id
        assert_eq!(encoded[encoded.len() - 1], 2); // ordinal
        let mut b = ByteBuffer::new(encoded);
        let decoded = decode_ok(&mut b);
        assert_eq!(decoded.components.rarity(), Some(ItemRarity::Rare));
    }

    #[test]
    fn minimum_attack_charge_roundtrip() {
        let mut item = ItemStack::new(264, 1);
        item.components
            .set(ComponentValue::MinimumAttackCharge(1.5));
        let mut b = ByteBuffer::with_capacity(8);
        encode_ok(&item, &mut b);
        let mut b = ByteBuffer::new(b.into_inner());
        let decoded = decode_ok(&mut b);
        assert_eq!(decoded.components.minimum_attack_charge(), Some(1.5));
    }

    #[test]
    fn unknown_component_id_rejected() {
        // 手工构造：count=1, material=264, added=1, removed=0, id=5(use_effects)
        let mut b = ByteBuffer::new(Vec::new());
        b.put_varint(1);
        b.put_varint(264);
        b.put_varint(1);
        b.put_varint(0);
        b.put_varint(5);
        b.put_varint(0); // value 多余字节不会读到
        let mut b = ByteBuffer::new(b.into_inner());
        assert_eq!(
            decode_item_stack(&mut b),
            Err(ProtocolError::UnsupportedComponents)
        );
    }

    #[test]
    fn truncated_value_returns_eof() {
        // count=1, material=264, added=1, removed=0, id=7, 仅 2 字节 f32
        let mut b = ByteBuffer::new(Vec::new());
        b.put_varint(1);
        b.put_varint(264);
        b.put_varint(1);
        b.put_varint(0);
        b.put_varint(7);
        b.put_varint(0x00); // 仅 2 字节，不足 f32 的 4 字节
        b.put_varint(0x00);
        let mut b = ByteBuffer::new(b.into_inner());
        assert_eq!(decode_item_stack(&mut b), Err(ProtocolError::UnexpectedEof));
    }

    #[test]
    fn added_removed_overflow_rejected() {
        // added=300 超出 256 上限 → InvalidValue
        let mut b = ByteBuffer::new(Vec::new());
        b.put_varint(1);
        b.put_varint(264);
        b.put_varint(300);
        b.put_varint(0);
        let mut b = ByteBuffer::new(b.into_inner());
        assert_eq!(decode_item_stack(&mut b), Err(ProtocolError::InvalidValue));
    }

    #[test]
    fn max_stack_default_when_no_component() {
        assert_eq!(ItemStack::new(264, 1).max_stack(), 64);
    }

    #[test]
    fn max_stack_uses_component() {
        let mut item = ItemStack::new(264, 1);
        item.components.set(ComponentValue::MaxStackSize(16));
        assert_eq!(item.max_stack(), 16);
    }

    // ── C 档：简单自定界全量 roundtrip ──

    /// 测试辅助：设置单个组件后整栈 roundtrip，返回解码后的物品。
    fn one_component_roundtrip(value: ComponentValue) -> ItemStack {
        let mut item = ItemStack::new(264, 1);
        item.components.set(value);
        let mut b = ByteBuffer::with_capacity(128);
        encode_ok(&item, &mut b);
        let mut b = ByteBuffer::new(b.into_inner());
        let decoded = decode_ok(&mut b);
        assert_eq!(decoded, item);
        decoded
    }

    #[test]
    fn byte_component_roundtrip() {
        let decoded = one_component_roundtrip(ComponentValue::Byte(-5));
        assert_eq!(decoded.components.get(22), Some(&ComponentValue::Byte(-5)));
    }

    #[test]
    fn short_component_roundtrip() {
        let decoded = one_component_roundtrip(ComponentValue::Short(-300));
        assert_eq!(
            decoded.components.get(45),
            Some(&ComponentValue::Short(-300))
        );
    }

    #[test]
    fn long_component_roundtrip() {
        let decoded = one_component_roundtrip(ComponentValue::Long(-9_000_000_000));
        assert_eq!(
            decoded.components.get(76),
            Some(&ComponentValue::Long(-9_000_000_000))
        );
    }

    #[test]
    fn float_component_roundtrip() {
        let decoded = one_component_roundtrip(ComponentValue::Float(1.5));
        assert_eq!(
            decoded.components.get(50),
            Some(&ComponentValue::Float(1.5))
        );
    }

    #[test]
    fn double_component_roundtrip() {
        let decoded = one_component_roundtrip(ComponentValue::Double(123.456));
        assert_eq!(
            decoded.components.get(77),
            Some(&ComponentValue::Double(123.456))
        );
    }

    #[test]
    fn string_component_roundtrip() {
        let decoded = one_component_roundtrip(ComponentValue::String("hello 世界".to_string()));
        assert_eq!(
            decoded.components.get(10),
            Some(&ComponentValue::String("hello 世界".to_string()))
        );
    }

    #[test]
    fn bool_component_roundtrip() {
        let t = one_component_roundtrip(ComponentValue::Bool(true));
        assert_eq!(t.components.get(21), Some(&ComponentValue::Bool(true)));
        let f = one_component_roundtrip(ComponentValue::Bool(false));
        assert_eq!(f.components.get(21), Some(&ComponentValue::Bool(false)));
    }

    // ── C 档：NBT 与文本承载 roundtrip ──

    #[test]
    fn custom_data_nbt_roundtrip() {
        let tag = NbtTag::Compound(vec![
            ("count".to_string(), NbtTag::Int(3)),
            (
                "display".to_string(),
                NbtTag::Compound(vec![(
                    "color".to_string(),
                    NbtTag::String("red".to_string()),
                )]),
            ),
        ]);
        let decoded = one_component_roundtrip(ComponentValue::Nbt(tag.clone()));
        assert_eq!(decoded.components.custom_data(), Some(&tag));
    }

    #[test]
    fn custom_name_text_roundtrip() {
        let c = Component::Text {
            text: "测试剑".to_string(),
            style: crate::text_component::Style {
                color: Some(0xFF_FF_00_00),
                italic: true,
                bold: false,
                ..Default::default()
            },
        };
        let decoded = one_component_roundtrip(ComponentValue::Text(c.clone()));
        assert_eq!(decoded.components.custom_name(), Some(&c));
    }

    #[test]
    fn lore_text_list_roundtrip() {
        let lore = vec![
            Component::text("第一行"),
            Component::Translatable {
                key: "item.lore.hint".to_string(),
                fallback: None,
                args: vec![Component::text("!")],
            },
        ];
        let decoded = one_component_roundtrip(ComponentValue::TextList(lore.clone()));
        assert_eq!(decoded.components.lore(), Some(lore.as_slice()));
    }

    #[test]
    fn enchantments_nbt_roundtrip() {
        // id=13 解码为 Nbt，且重编码保留原 id（custom_data(0) 不冲突）。
        let tag = NbtTag::Compound(vec![(
            "enchantments".to_string(),
            NbtTag::List(vec![NbtTag::Compound(vec![
                (
                    "id".to_string(),
                    NbtTag::String("minecraft:sharpness".to_string()),
                ),
                ("lvl".to_string(), NbtTag::Short(5)),
            ])]),
        )]);
        // 手工构造 id=13 的 Set 条目（模拟解码自对端后重编码）。
        let mut item = ItemStack::new(264, 1);
        item.components.entries.push(ComponentEntry::Set {
            id: 13,
            value: ComponentValue::Nbt(tag.clone()),
        });
        let mut b = ByteBuffer::with_capacity(128);
        encode_ok(&item, &mut b);
        let mut b = ByteBuffer::new(b.into_inner());
        let decoded = decode_ok(&mut b);
        assert_eq!(decoded, item);
        assert_eq!(decoded.components.enchantments(), Some(&tag));
        // get(0) 应返回 None（custom_data 未设置），证明 id 13 不冲突。
        assert_eq!(decoded.components.get(0), None);
    }

    #[test]
    fn text_component_ids_are_stable_on_reencode() {
        // set(Text) 写入 id=6；set(Nbt) 写入 id=0；set(TextList) 写入 id=11。
        let mut item = ItemStack::new(264, 1);
        item.components
            .set(ComponentValue::Text(Component::text("n")));
        item.components
            .set(ComponentValue::Nbt(NbtTag::Compound(vec![])));
        item.components
            .set(ComponentValue::TextList(vec![Component::text("l")]));
        let mut b = ByteBuffer::with_capacity(128);
        encode_ok(&item, &mut b);
        let mut b = ByteBuffer::new(b.into_inner());
        let decoded = decode_ok(&mut b);
        assert_eq!(decoded, item);
        assert!(decoded.components.custom_name().is_some());
        assert!(decoded.components.custom_data().is_some());
        assert!(decoded.components.lore().is_some());
    }

    #[test]
    fn malformed_text_component_rejected() {
        // id=6 的值声明为 NBT Compound 但 `text` 键类型错误 → InvalidValue。
        let mut b = ByteBuffer::new(Vec::new());
        b.put_varint(1);
        b.put_varint(264);
        b.put_varint(1);
        b.put_varint(0);
        b.put_varint(6); // custom_name
        b.put_u8(0x0a);
        // anonymous Compound：TAG_Int(3) + name "text" + int(7) + TAG_End
        b.put_u8(0x03);
        b.put_varint(4);
        b.put_bytes("text".as_bytes());
        b.put_i32(7);
        b.put_u8(0x00);
        let mut b = ByteBuffer::new(b.into_inner());
        assert_eq!(decode_item_stack(&mut b), Err(ProtocolError::InvalidValue));
    }

    #[test]
    fn truncated_nbt_component_rejected() {
        // id=0(custom_data) 声明 0x0a 后无 payload → UnexpectedEof。
        let mut b = ByteBuffer::new(Vec::new());
        b.put_varint(1);
        b.put_varint(264);
        b.put_varint(1);
        b.put_varint(0);
        b.put_varint(0);
        b.put_u8(0x0a); // 后无字节
        let mut b = ByteBuffer::new(b.into_inner());
        assert_eq!(decode_item_stack(&mut b), Err(ProtocolError::UnexpectedEof));
    }

    #[test]
    fn lore_count_overflow_rejected() {
        // id=11(lore) 声明 300 个组件，超出 COMPONENT.list(256) 上限。
        let mut b = ByteBuffer::new(Vec::new());
        b.put_varint(1);
        b.put_varint(264);
        b.put_varint(1);
        b.put_varint(0);
        b.put_varint(11);
        b.put_varint(300);
        let mut b = ByteBuffer::new(b.into_inner());
        assert_eq!(decode_item_stack(&mut b), Err(ProtocolError::InvalidValue));
    }
}
