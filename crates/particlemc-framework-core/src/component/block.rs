//! 方块类型：对「方块状态 id」的轻量值类型封装。
//!
//! [`Block`] 以 `Copy` 值类型承载一个方块状态 id，并提供经注册表解析的
//! 名称查询、空气 / 实心判定与属性查询。它只持有数值，所有语义查询
//! 均委托给 [`BlockRegistry`]，因此可在系统中按值传递而无需借用注册表。

use crate::resource::registries::BlockRegistry;

/// 方块：封装一个方块状态 id 的轻量值类型。
///
/// 方块在 Minecraft 中以全局状态 id 唯一标识（如 `minecraft:stone` 通常为 1、
/// `minecraft:air` 为 0）。本类型不持有注册表引用，语义查询需显式传入
/// [`BlockRegistry`]。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Block {
    /// 方块状态 id（`pub(crate)` 可见，供注册表内部互操作）。
    pub(crate) state_id: u32,
}

impl Block {
    /// 由方块状态 id 直接构造方块。
    pub fn from_state_id(state_id: u32) -> Self {
        Self { state_id }
    }

    /// 按命名空间名称从注册表解析方块；未注册时返回 `None`。
    pub fn from_name(registry: &BlockRegistry, name: &str) -> Option<Self> {
        registry.block_from_name(name)
    }

    /// 返回方块状态 id。
    pub fn state_id(self) -> u32 {
        self.state_id
    }

    /// 按 id 反查命名空间名称；未注册时返回 `None`。
    pub fn name(self, registry: &BlockRegistry) -> Option<&str> {
        registry.name_of(self.state_id)
    }

    /// 是否为空气方块（id 等于注册表的 `minecraft:air`）。
    pub fn is_air(self, registry: &BlockRegistry) -> bool {
        self.state_id == registry.air_id()
    }

    /// 是否实心（缺省语义：非空气即实心）。
    pub fn is_solid(self, registry: &BlockRegistry) -> bool {
        registry.is_solid(self.state_id)
    }

    /// 查询方块属性值；当前数据源未产出状态表，恒返回 `None`。
    ///
    /// 保留该 API 形状，后续数据源补充状态属性时可直接在此取值。
    /// 返回值借用自 `registry`（属性定义存放于方块定义中）。
    pub fn property<'a>(self, registry: &'a BlockRegistry, key: &str) -> Option<&'a str> {
        // 经 BlockDefinition 查找方块定义；states 表当前为 None，一律返回 None。
        let definition = registry.0.get(self.state_id)?;
        let states = definition.states.as_ref()?;
        let _ = (states, key);
        None
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::resource::registries::{BlockDefinition, Registry};

    /// 构造最小测试注册表：air=0、stone=1。
    fn test_registry() -> BlockRegistry {
        let toml = r#"
            [[entry]]
            id = 0
            name = "minecraft:air"

            [[entry]]
            id = 1
            name = "minecraft:stone"
        "#;
        let inner = Registry::<BlockDefinition>::from_toml_str(toml).unwrap();
        BlockRegistry(inner)
    }

    #[test]
    fn stone_constructs_name_and_solid() {
        let registry = test_registry();
        let stone = Block::from_state_id(1);
        assert_eq!(stone.state_id(), 1);
        assert_eq!(stone.name(&registry), Some("minecraft:stone"));
        assert!(stone.is_solid(&registry));
        assert!(!stone.is_air(&registry));
    }

    #[test]
    fn air_is_recognized() {
        let registry = test_registry();
        let air = Block::from_state_id(0);
        assert!(air.is_air(&registry));
        assert!(!air.is_solid(&registry));
        assert_eq!(air.name(&registry), Some("minecraft:air"));
    }

    #[test]
    fn from_name_resolves_block() {
        let registry = test_registry();
        let stone = Block::from_name(&registry, "minecraft:stone").unwrap();
        assert_eq!(stone.state_id(), 1);
        assert!(Block::from_name(&registry, "minecraft:missing").is_none());
    }

    #[test]
    fn state_id_roundtrips() {
        let registry = test_registry();
        let block = Block::from_name(&registry, "minecraft:stone").unwrap();
        assert_eq!(Block::from_state_id(block.state_id()), block);
        assert_eq!(block.state_id(), 1);
    }

    #[test]
    fn property_returns_none_with_current_data() {
        let registry = test_registry();
        let stone = Block::from_state_id(1);
        // 当前数据源无状态表，任何属性查询均返回 None。
        assert_eq!(stone.property(&registry, "waterlogged"), None);
    }
}
