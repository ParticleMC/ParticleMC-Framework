//! 方块形状查询：由方块状态 id 解析碰撞形状。
//!
//! [`block_shape`] 把「方块状态 id + 注册表」映射为 [`Shape`]：
//! - 实心方块查 [`crate::physics::block_shapes::shape_boxes`] 表：
//!   普通方块返回单位 [`Shape::Aabb`]，slab / stair 返回 [`Shape::Merged`]。
//! - 空气等非实心返回 [`Shape::Empty`]。
//!
//! 非单位形状的几何数据集中在 [`crate::physics::block_shapes`]，本模块只负责
//! 「id → 名 → 形状」的查询与失败兜底，便于统一维护形状来源。
//!
//! 变更标识符：`complete-framework-gaps`（WS3）。
//! 见 [`.specs/complete-framework-gaps/spec.md`]。

use crate::resource::registries::BlockRegistry;

use super::Aabb;
use super::block_shapes::{Box6, box6_to_aabb, shape_boxes};
use super::shape::Shape;

/// 单位方块碰撞盒（`[0,0,0]` 到 `[1,1,1]`）。
fn unit_block_shape() -> Shape {
    Shape::Aabb(Aabb::from_pos_size([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]))
}

/// 按方块状态 id 解析碰撞形状。
///
/// - 注册表无此 id → 兜底为实心单位 AABB（安全默认，避免未知方块穿透）。
/// - 非实心（如空气）→ [`Shape::Empty`]。
/// - 实心 → 查形状表 [`shape_boxes`]：单盒且为单位立方体时返回
///   [`Shape::Aabb`]（普通方块快速路径）；单盒但非单位（如 slab）或多盒
///   （如 stair）返回 [`Shape::Merged`]。几何判定保证任何非单位形状都归入
///   `Merged`，与碰撞逻辑（按实际包围盒求交）保持一致。
///
/// 形状表未命中（返回 `None`）时按 [`Shape::Empty`] 处理（例如空气被误判为实心）。
#[must_use]
pub fn block_shape(block_id: u32, registry: &BlockRegistry) -> Shape {
    // 未注册 id 兜底为实心（安全默认，见函数文档）。
    if registry.0.get(block_id).is_none() {
        return unit_block_shape();
    }
    if !registry.is_solid(block_id) {
        return Shape::Empty;
    }
    // 实心方块：按名称查形状表，返回单盒（Aabb）或多盒（Merged）。
    let name = match registry.name_of(block_id) {
        Some(name) => name,
        // 已注册但查不到名称（理论不会发生）：兜底单位盒。
        None => return unit_block_shape(),
    };
    match shape_boxes(name) {
        None => Shape::Empty,
        Some(boxes) if boxes.is_empty() => Shape::Empty,
        Some(boxes) if boxes.len() == 1 => {
            // 单盒：单位立方体按普通实心块归入 `Aabb`（快速路径）；
            // 非单位单盒（如 slab）按非单位形状归入 `Merged`，保持几何一致。
            if is_unit_box(&boxes[0]) {
                Shape::Aabb(box6_to_aabb(&boxes[0]))
            } else {
                Shape::Merged(vec![box6_to_aabb(&boxes[0])])
            }
        }
        Some(boxes) => Shape::Merged(boxes.iter().map(box6_to_aabb).collect()),
    }
}

/// 判断六元组是否为单位立方体 `[0,1)³`（精确字面比较，无浮点误差）。
#[must_use]
fn is_unit_box(box6: &Box6) -> bool {
    *box6 == [0.0, 0.0, 0.0, 1.0, 1.0, 1.0]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::resource::registries::{BlockDefinition, Registry};

    /// 构造最小方块注册表：air=0、stone=1、water=2（当前 is_solid 语义下
    /// 非空气即实心，water 亦为实心，见模块文档说明）。
    fn test_block_registry() -> BlockRegistry {
        let toml = r#"
            [[entry]]
            id = 0
            name = "minecraft:air"

            [[entry]]
            id = 1
            name = "minecraft:stone"

            [[entry]]
            id = 2
            name = "minecraft:water"
        "#;
        let inner = Registry::<BlockDefinition>::from_toml_str(toml).unwrap();
        BlockRegistry(inner)
    }

    #[test]
    fn solid_block_is_unit_aabb() {
        let registry = test_block_registry();
        let shape = block_shape(1, &registry);
        // 单位方块：与内部点相交、与 [0,1)³ 重叠。
        assert!(shape.intersects(&Aabb::from_pos_size([0.5, 0.5, 0.5], [0.5, 0.5, 0.5])));
        assert!(shape.overlaps_block(0, 0, 0));
        assert!(!shape.is_empty());
        assert_eq!(shape, unit_block_shape());
    }

    #[test]
    fn air_is_empty() {
        let registry = test_block_registry();
        let shape = block_shape(0, &registry);
        assert!(shape.is_empty());
        assert!(!shape.intersects(&Aabb::from_pos_size([0.5, 0.5, 0.5], [0.5, 0.5, 0.5])));
        assert!(!shape.overlaps_block(0, 0, 0));
    }

    #[test]
    fn unknown_id_defaults_to_solid() {
        // 未注册 id（999）兜底为实心：返回单位 AABB 而非 Empty。
        let registry = test_block_registry();
        let shape = block_shape(999, &registry);
        assert!(!shape.is_empty());
        assert!(shape.overlaps_block(0, 0, 0));
        assert_eq!(shape, unit_block_shape());
    }

    #[test]
    fn merged_shape_result_intersects() {
        // 组合多个单位 AABB 的 Merged 形状与方块重叠查询一致。
        let a = Aabb::from_pos_size([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let b = Aabb::from_pos_size([10.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let merged = Shape::merged([a, b]);
        assert!(merged.overlaps_block(10, 0, 0));
        assert!(merged.intersects(&Aabb::from_pos_size([10.5, 0.5, 0.5], [0.1, 0.1, 0.1])));
        assert!(!merged.overlaps_block(5, 0, 0));
    }

    /// 构造含 slab / stair 形状名称的最小注册表（air=0、stone=1、
    /// lower_slab=2、upper_slab=3、stair=4），用于验证非单位形状查询。
    fn shaped_block_registry() -> BlockRegistry {
        let toml = r#"
            [[entry]]
            id = 0
            name = "minecraft:air"

            [[entry]]
            id = 1
            name = "minecraft:stone"

            [[entry]]
            id = 2
            name = "minecraft:oak_slab"

            [[entry]]
            id = 3
            name = "minecraft:oak_slab_top"

            [[entry]]
            id = 4
            name = "minecraft:oak_stairs"
        "#;
        let inner = Registry::<BlockDefinition>::from_toml_str(toml).unwrap();
        BlockRegistry(inner)
    }

    #[test]
    fn lower_slab_is_merged_with_top_at_half() {
        let registry = shaped_block_registry();
        let shape = block_shape(2, &registry);
        match shape {
            Shape::Merged(boxes) => {
                assert_eq!(boxes.len(), 1);
                // 下半 slab：占据 [0, 0.5)，顶面在 y = 0.5。
                assert_eq!(boxes[0].min, [0.0, 0.0, 0.0]);
                assert_eq!(boxes[0].max, [1.0, 0.5, 1.0]);
            }
            other => panic!("预期 Merged，得到 {other:?}"),
        }
    }

    #[test]
    fn upper_slab_is_merged_top_half_only() {
        let registry = shaped_block_registry();
        let shape = block_shape(3, &registry);
        match shape {
            Shape::Merged(boxes) => {
                assert_eq!(boxes.len(), 1);
                // 上半 slab：占据 [0.5, 1)，仅上沿阻挡。
                assert_eq!(boxes[0].min, [0.0, 0.5, 0.0]);
                assert_eq!(boxes[0].max, [1.0, 1.0, 1.0]);
            }
            other => panic!("预期 Merged，得到 {other:?}"),
        }
    }

    #[test]
    fn stair_is_merged_with_two_boxes() {
        let registry = shaped_block_registry();
        let shape = block_shape(4, &registry);
        match shape {
            Shape::Merged(boxes) => {
                assert_eq!(boxes.len(), 2);
                // 下半整块 + 上半后半个。
                assert_eq!(boxes[0].min, [0.0, 0.0, 0.0]);
                assert_eq!(boxes[0].max, [1.0, 0.5, 1.0]);
                assert_eq!(boxes[1].min, [0.0, 0.5, 0.5]);
                assert_eq!(boxes[1].max, [1.0, 1.0, 1.0]);
            }
            other => panic!("预期 Merged，得到 {other:?}"),
        }
    }

    #[test]
    fn solid_still_unit_aabb_after_shape_table() {
        let registry = shaped_block_registry();
        assert_eq!(block_shape(1, &registry), unit_block_shape());
    }
}
