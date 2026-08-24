//! 实体类型：对「实体类型注册表 id」的轻量值类型封装。
//!
//! [`EntityType`] 以 `Copy` 值类型承载一个实体类型注册表 id，并提供经
//! [`EntityTypeRegistry`] 解析的名称与尺寸查询。它只持有数值，所有语义查询
//! 均委托给注册表，因此可在系统中按值传递而无需持有注册表引用。

use crate::resource::registries::EntityTypeRegistry;

/// 默认实体宽度（注册表未提供时的回退值）。
const DEFAULT_WIDTH: f64 = 0.6;
/// 默认实体高度（注册表未提供时的回退值）。
const DEFAULT_HEIGHT: f64 = 1.8;

/// 实体类型：封装一个实体类型注册表 id 的轻量值类型。
///
/// 实体类型在 Minecraft 中以注册表序号唯一标识（如 `minecraft:player`、
/// `minecraft:cow`）。本类型不持有注册表引用，语义查询需显式传入
/// [`EntityTypeRegistry`]。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct EntityType {
    /// 注册表序号（`pub(crate)` 可见，供注册表内部互操作）。
    pub(crate) id: u32,
}

impl EntityType {
    /// 按命名空间名称从注册表解析实体类型；未注册时返回 `None`。
    pub fn by_name(registry: &EntityTypeRegistry, name: &str) -> Option<Self> {
        registry.0.get_id(name).map(|id| Self { id })
    }

    /// 由注册表序号直接构造实体类型（不校验 id，恒返回）。
    pub fn by_id(id: u32) -> Self {
        Self { id }
    }

    /// 返回注册表序号。
    pub fn id(self) -> u32 {
        self.id
    }

    /// 按序号反查命名空间名称；未注册时返回 `None`。
    pub fn name(self, registry: &EntityTypeRegistry) -> Option<&str> {
        registry.0.get_name(self.id)
    }

    /// 实体宽度（注册表未提供时回退为 0.6）。
    pub fn width(self, registry: &EntityTypeRegistry) -> f64 {
        registry
            .0
            .get(self.id)
            .and_then(|def| def.width)
            .unwrap_or(DEFAULT_WIDTH)
    }

    /// 实体高度（注册表未提供时回退为 1.8）。
    pub fn height(self, registry: &EntityTypeRegistry) -> f64 {
        registry
            .0
            .get(self.id)
            .and_then(|def| def.height)
            .unwrap_or(DEFAULT_HEIGHT)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::resource::registries::{EntityTypeDefinition, Registry};

    /// 构造最小测试注册表：player=0、cow=1、shulker=2（无尺寸字段）。
    fn test_registry() -> EntityTypeRegistry {
        let toml = r#"
            [[entry]]
            id = 0
            name = "minecraft:player"
            width = 0.6
            height = 1.8

            [[entry]]
            id = 1
            name = "minecraft:cow"
            width = 0.9
            height = 1.4

            [[entry]]
            id = 2
            name = "minecraft:shulker"
        "#;
        let inner = Registry::<EntityTypeDefinition>::from_toml_str(toml).unwrap();
        EntityTypeRegistry(inner)
    }

    #[test]
    fn by_name_resolves_and_id_roundtrips() {
        let registry = test_registry();
        let cow = EntityType::by_name(&registry, "minecraft:cow").unwrap();
        assert_eq!(cow.id(), 1);
        assert_eq!(EntityType::by_name(&registry, "minecraft:missing"), None);
        // by_id ↔ id 互为逆映射。
        assert_eq!(EntityType::by_id(cow.id()), cow);
    }

    #[test]
    fn by_id_is_unchecked() {
        // 任意 id 均返回实体类型，不校验注册表。
        let ty = EntityType::by_id(999);
        assert_eq!(ty.id(), 999);
    }

    #[test]
    fn name_reverse_lookup() {
        let registry = test_registry();
        let player = EntityType::by_id(0);
        assert_eq!(player.name(&registry), Some("minecraft:player"));
        assert_eq!(EntityType::by_id(999).name(&registry), None);
    }

    #[test]
    fn width_height_from_registry() {
        let registry = test_registry();
        let cow = EntityType::by_id(1);
        assert_eq!(cow.width(&registry), 0.9);
        assert_eq!(cow.height(&registry), 1.4);
    }

    #[test]
    fn width_height_fallback_when_missing() {
        let registry = test_registry();
        // shulker 未提供尺寸字段，回退到默认 0.6 / 1.8。
        let shulker = EntityType::by_id(2);
        assert_eq!(shulker.width(&registry), 0.6);
        assert_eq!(shulker.height(&registry), 1.8);
        // 未注册的 id 同样回退到默认。
        let unknown = EntityType::by_id(999);
        assert_eq!(unknown.width(&registry), 0.6);
        assert_eq!(unknown.height(&registry), 1.8);
    }
}
