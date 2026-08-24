//! 方块状态组件。
//!
//! 记录某位置方块的数值 `block_id` 与一组键值属性。属性以固定长度数组
//! `props: [Option<u8>; 16]` 存储，槽位索引由注册表侧（`BlockStateDef`）分配。
//! `BlockState` 为 `Clone` / `Default`，用于世界中块状态的可查询快照。
//!
//! See `.specs/optimize-block-interaction-and-chunk-store/spec.md`（T2）。

use crate::prelude::Component;
use crate::resource::registries::BlockRegistry;

/// 方块状态：方块 id + 属性槽位数组。
///
/// `props[i]` 存储属性槽位 `i` 的值（`Some(u8)`）或空（`None`）。
/// 槽位索引由注册表侧 [`BlockStateDef::property_indices`] 定义，调用方应先通过
/// [`BlockRegistry`] 查询键对应的槽位，再使用索引访问数组。
#[derive(Default, Component, Debug, Clone, PartialEq, Eq)]
#[component(storage = "sparse")]
pub struct BlockState {
    /// 方块数值 id。
    pub block_id: u32,
    /// 方块属性槽位数组（最多 16 个槽位）。
    ///
    /// 槽位由注册表侧的 [`BlockStateDef::property_indices`] 分配，
    /// 未写入的槽位为 `None`。
    pub props: [Option<u8>; 16],
}

impl BlockState {
    /// 以方块 id 构造（所有槽位为空）。
    pub fn from_id(block_id: u32) -> Self {
        Self {
            block_id,
            props: [None; 16],
        }
    }

    /// 返回方块数值 id。
    pub fn block_id(&self) -> u32 {
        self.block_id
    }

    /// 按槽位索引读取属性值。
    ///
    /// 索引在 `0..16` 范围内有效；超出范围返回 `None`。
    /// 槽位已写入时返回 `Some(value)`，未写入时返回 `None`。
    pub fn get_property_by_index(&self, index: usize) -> Option<u8> {
        if index < self.props.len() {
            self.props[index]
        } else {
            None
        }
    }

    /// 按槽位索引写入属性值。
    ///
    /// 索引在 `0..16` 范围内有效；超出范围返回 `Err(index)`。
    /// 返回 `Ok(())` 表示写入成功。
    pub fn set_property_by_index(&mut self, index: usize, value: u8) -> Result<(), usize> {
        if index < self.props.len() {
            self.props[index] = Some(value);
            Ok(())
        } else {
            Err(index)
        }
    }

    /// 查询某个属性名称对应的槽位值。
    ///
    /// 该方法遍历注册表中所有方块定义，查找第一个含有目标属性名的
    /// [`BlockStateDef::property_indices`](crate::resource::registries::BlockStateDef::property_indices)
    /// 映射，获取槽位索引后委托给
    /// [`get_property_by_index`](Self::get_property_by_index) 读取数组。
    /// 若未找到属性映射或对应槽位未写入，返回 `None`。
    pub fn get_property(&self, registry: &BlockRegistry, property_name: &str) -> Option<u8> {
        for def in registry.0.values() {
            if let Some(ref states) = def.states {
                for state_def in states.values() {
                    if let Some(&index) = state_def.property_indices.get(property_name) {
                        return self.get_property_by_index(index as usize);
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::resource::registries::{BlockDefinition, BlockRegistry};
    use std::collections::HashMap;

    #[test]
    fn from_id_construction() {
        let state = BlockState::from_id(42);
        assert_eq!(state.block_id(), 42);
        assert_eq!(state.props, [None; 16]);
    }

    #[test]
    fn default_is_all_none() {
        let state = BlockState::default();
        assert_eq!(state.props, [None; 16]);
    }

    #[test]
    fn get_property_by_index_reads_written_slot() {
        let mut state = BlockState::from_id(1);
        state.set_property_by_index(0, 5).unwrap();
        assert_eq!(state.get_property_by_index(0), Some(5));
    }

    #[test]
    fn get_property_by_index_returns_none_for_empty_slot() {
        let state = BlockState::from_id(1);
        assert_eq!(state.get_property_by_index(0), None);
    }

    #[test]
    fn get_property_by_index_returns_none_out_of_bounds() {
        let state = BlockState::from_id(1);
        assert_eq!(state.get_property_by_index(16), None);
        assert_eq!(state.get_property_by_index(99), None);
    }

    #[test]
    fn set_property_by_index_writes_and_overwrites() {
        let mut state = BlockState::from_id(1);
        // 首次写入。
        state.set_property_by_index(3, 7).unwrap();
        assert_eq!(state.get_property_by_index(3), Some(7));
        // 覆盖写入。
        state.set_property_by_index(3, 9).unwrap();
        assert_eq!(state.get_property_by_index(3), Some(9));
    }

    #[test]
    fn set_property_by_index_returns_err_out_of_bounds() {
        let mut state = BlockState::from_id(1);
        assert_eq!(state.set_property_by_index(16, 0), Err(16));
        assert_eq!(state.set_property_by_index(99, 0), Err(99));
    }

    #[test]
    fn clone_preserves_properties() {
        let mut state = BlockState::from_id(1);
        state.set_property_by_index(0, 3).unwrap();
        state.set_property_by_index(7, 11).unwrap();
        let cloned = state.clone();
        assert_eq!(cloned.get_property_by_index(0), Some(3));
        assert_eq!(cloned.get_property_by_index(7), Some(11));
        assert_eq!(cloned.block_id(), 1);
    }

    #[test]
    fn partial_eq_matches_props() {
        let mut a = BlockState::from_id(1);
        let mut b = BlockState::from_id(1);
        let mut c = BlockState::from_id(1);
        a.set_property_by_index(0, 5).unwrap();
        b.set_property_by_index(0, 5).unwrap();
        c.set_property_by_index(0, 6).unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn get_property_reads_via_registry_lookup() {
        // 构造一个含 states / property_indices 的最小注册表。
        let reg: crate::resource::registries::Registry<BlockDefinition> = {
            let mut r = crate::resource::registries::Registry::new();
            let def = BlockDefinition {
                name: "minecraft:lever".to_string(),
                id: Some(5),
                translation_key: None,
                hardness: None,
                default_state_id: None,
                states: Some({
                    let mut m = HashMap::new();
                    m.insert(
                        "on".to_string(),
                        crate::resource::registries::BlockStateDef {
                            id: Some(0),
                            property_indices: {
                                let mut p = HashMap::new();
                                p.insert("face".to_string(), 2);
                                p
                            },
                            extra: HashMap::new(),
                        },
                    );
                    m
                }),
                light_opacity: 0,
                light_emission: 0,
                extra: HashMap::new(),
            };
            r.register("minecraft:lever", def).unwrap();
            r
        };
        let registry = BlockRegistry(reg);

        let mut state = BlockState::from_id(5);
        state.set_property_by_index(2, 7).unwrap();
        assert_eq!(state.get_property(&registry, "face"), Some(7));
    }

    #[test]
    fn get_property_returns_none_when_property_not_found() {
        let reg: crate::resource::registries::Registry<BlockDefinition> = {
            let mut r = crate::resource::registries::Registry::new();
            let def = BlockDefinition {
                name: "minecraft:stone".to_string(),
                id: Some(1),
                translation_key: None,
                hardness: None,
                default_state_id: None,
                states: None,
                light_opacity: 15,
                light_emission: 0,
                extra: HashMap::new(),
            };
            r.register("minecraft:stone", def).unwrap();
            r
        };
        let registry = BlockRegistry(reg);

        let state = BlockState::from_id(1);
        assert_eq!(state.get_property(&registry, "waterlogged"), None);
    }

    #[test]
    fn get_property_returns_none_when_slot_empty() {
        let reg: crate::resource::registries::Registry<BlockDefinition> = {
            let mut r = crate::resource::registries::Registry::new();
            let def = BlockDefinition {
                name: "minecraft:lever".to_string(),
                id: Some(5),
                translation_key: None,
                hardness: None,
                default_state_id: None,
                states: Some({
                    let mut m = HashMap::new();
                    m.insert(
                        "on".to_string(),
                        crate::resource::registries::BlockStateDef {
                            id: Some(0),
                            property_indices: {
                                let mut p = HashMap::new();
                                p.insert("face".to_string(), 0);
                                p
                            },
                            extra: HashMap::new(),
                        },
                    );
                    m
                }),
                light_opacity: 0,
                light_emission: 0,
                extra: HashMap::new(),
            };
            r.register("minecraft:lever", def).unwrap();
            r
        };
        let registry = BlockRegistry(reg);

        // 槽位 0 未写入，应返回 None。
        let state = BlockState::from_id(5);
        assert_eq!(state.get_property(&registry, "face"), None);
    }
}
