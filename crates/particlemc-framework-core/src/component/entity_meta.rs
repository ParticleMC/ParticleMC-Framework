//! 实体元数据组件：实体类型 + 按协议 index 组织的有序元数据表。
//!
//! [`EntityMeta`] 记录实体类型（[`EntityType`]）与实体元数据键值表
//! （[`EntityMetadataMap`]）。元数据值对应 Minecraft 协议 Entity Metadata
//! 条目（如 index 8 = flags 用 [`EntityMetadataValue::Byte`]、
//! index 2 = custom_name 用 [`EntityMetadataValue::String`]（JSON 文本）、
//! index 9 = health 用 [`EntityMetadataValue::Float`]），用于向客户端同步
//! 实体的外观 / 状态。

use crate::prelude::Component;

use std::collections::BTreeMap;

use crate::resource::EntityType;

/// 实体元数据组件。
#[derive(Component, Debug, Clone, Default)]
#[component(storage = "sparse")]
pub struct EntityMeta {
    /// 实体类型（未设置时为 `None`）。
    pub entity_type: Option<EntityType>,
    /// 元数据键值表（按 index 有序）。
    pub data: EntityMetadataMap,
}

impl EntityMeta {
    /// 以实体类型构造元数据组件（元数据表为空）。
    pub fn new(entity_type: EntityType) -> Self {
        Self {
            entity_type: Some(entity_type),
            data: EntityMetadataMap::default(),
        }
    }
}

/// 单个实体元数据条目的值（协议值类型 v1）。
#[derive(Debug, Clone, PartialEq)]
pub enum EntityMetadataValue {
    /// 字节值（线格式 BYTE）。
    Byte(u8),
    /// 变长整数（线格式 VARINT）。
    VarInt(i32),
    /// 单精度浮点（线格式 FLOAT）。
    Float(f32),
    /// 字符串（线格式 STRING：VarInt 长度 + UTF-8）。
    String(String),
    /// 布尔值（线格式 BOOLEAN）。
    Bool(bool),
}

/// 实体元数据表：按协议 `index` 有序存放的键值对集合。
///
/// 内部以 `BTreeMap<u32, EntityMetadataValue>` 承载，天然保持 index 升序，
/// 查询 / 插入均为 O(log n)。`set` 时若 index 已存在则直接更新，否则插入
/// 并保持有序；`get` / `iter` / `len` / `is_empty` 语义与 Vec 版本一致。
#[derive(Debug, Clone, Default)]
pub struct EntityMetadataMap {
    /// 有序条目（按 index 升序，由 BTreeMap 保证）。
    entries: BTreeMap<u32, EntityMetadataValue>,
}

impl EntityMetadataMap {
    /// 构造空表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 写入（或更新）指定 index 的元数据值，保持按 index 升序。
    ///
    /// 若 index 已存在则直接替换值；否则按 BTreeMap 有序性自动插入。
    pub fn set(&mut self, index: u32, value: EntityMetadataValue) {
        self.entries.insert(index, value);
    }

    /// 查询指定 index 的元数据值；不存在返回 `None`。
    pub fn get(&self, index: u32) -> Option<&EntityMetadataValue> {
        self.entries.get(&index)
    }

    /// 迭代全部条目（按 index 升序）。
    pub fn iter(&self) -> impl Iterator<Item = (&u32, &EntityMetadataValue)> {
        self.entries.iter()
    }

    /// 条目数量。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn set_keeps_order_and_updates_existing() {
        let mut map = EntityMetadataMap::new();
        assert!(map.is_empty());
        map.set(9, EntityMetadataValue::Float(20.0));
        map.set(2, EntityMetadataValue::String("hi".to_string()));
        map.set(8, EntityMetadataValue::Byte(0x02));
        // 乱序插入后仍按 index 升序。
        let indices: Vec<u32> = map.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![2, 8, 9]);
        assert_eq!(map.len(), 3);
        // 更新已有 index：数量不变、值被替换。
        map.set(8, EntityMetadataValue::Byte(0x04));
        assert_eq!(map.len(), 3);
        assert_eq!(map.get(8), Some(&EntityMetadataValue::Byte(0x04)));
    }

    #[test]
    fn get_returns_value_or_none() {
        let mut map = EntityMetadataMap::new();
        map.set(2, EntityMetadataValue::String("hi".to_string()));
        assert_eq!(
            map.get(2),
            Some(&EntityMetadataValue::String("hi".to_string()))
        );
        assert_eq!(map.get(3), None);
    }

    #[test]
    fn entity_meta_constructs_with_type() {
        let ty = EntityType::by_id(7);
        let meta = EntityMeta::new(ty);
        assert_eq!(meta.entity_type, Some(ty));
        assert!(meta.data.is_empty());
    }
}
