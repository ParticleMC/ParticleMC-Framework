// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 流体语义：方块状态 id → 流体描述。
//!
//! 语义对齐框架的 `Fluid` / `FluidImpl`：区分水 / 岩浆 / 空三种，
//! 并携带高度等级 `level`（0..=15，`level == 8` 表示下落态，v1 原样透传）。
//!
//! **注意（v1 限制）**：`BlockRegistry` 当前仅暴露 `is_solid` / `air_id` /
//! `name_of` 等查询，**无 `is_fluid` / `fluid` 接口**（数据源 `blocks.toml`
//! 的 `liquid` 属性在 `extra` 透传字段中，未建索引）。因此 `fluid_from_block`
//! 按注册表名称**包含 `water` / `lava` 前缀**判定流体类别，并对 `extra`
//! 中的 `level` 数值做尽力解析（缺失时取 0）。注册表缺该方块时视为非流体。
//!
//! 变更标识符：`complete-missing-subsystems`（T9/R9）。

use crate::resource::registries::BlockRegistry;

/// 流体描述：类别 + 等级。
///
/// `is_water` 与 `is_lava` 互斥（由构造器保证）；两者均 `false` 时为「空」
/// （`is_none()` 为 `true`），表示该方块不是流体。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Fluid {
    /// 是否为水。
    pub is_water: bool,
    /// 是否为岩浆。
    pub is_lava: bool,
    /// 流体等级（0..=15；v1 原样透传，不校验区间）。
    pub level: u8,
}

impl Fluid {
    /// 非流体（空）。
    pub fn none() -> Self {
        Self::default()
    }

    /// 水（等级 `level`，截断到 0..=15）。
    pub fn water(level: u8) -> Self {
        Self {
            is_water: true,
            is_lava: false,
            level: level.min(15),
        }
    }

    /// 岩浆（等级 `level`，截断到 0..=15）。
    pub fn lava(level: u8) -> Self {
        Self {
            is_water: false,
            is_lava: true,
            level: level.min(15),
        }
    }

    /// 是否为非流体。
    pub fn is_none(&self) -> bool {
        !self.is_water && !self.is_lava
    }
}

/// 由方块状态 id 解析流体描述。
///
/// 判定顺序：
/// 1. 注册表反查名称；未注册 → [`Fluid::none`]；
/// 2. 名称含 `water` → 水；含 `lava` → 岩浆；否则 → 非流体；
/// 3. 尝试从注册表 `extra.level` 解析等级（方块状态数据可能带 `level`），
///    缺失或非整数时取 0。
///
/// 名称判定为 v1 简化策略：`minecraft:water` / `minecraft:flowing_water`
/// 均判为水，`minecraft:lava` / `minecraft:flowing_lava` 均判为岩浆。
pub fn fluid_from_block(block_id: u32, registry: &BlockRegistry) -> Fluid {
    let Some(name) = registry.name_of(block_id) else {
        return Fluid::none();
    };
    let level = fluid_level(block_id, registry);
    if name.contains("water") {
        Fluid::water(level)
    } else if name.contains("lava") {
        Fluid::lava(level)
    } else {
        Fluid::none()
    }
}

/// 从注册表条目 `extra.level` 尽力解析流体等级；缺失/非整数时返回 0。
fn fluid_level(block_id: u32, registry: &BlockRegistry) -> u8 {
    let level = registry
        .0
        .get(block_id)
        .and_then(|def| def.extra.get("level"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    u8::try_from(level).unwrap_or(0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::resource::registries::{BlockDefinition, Registry};

    /// 构造测试注册表：air=0、water=1（extra.level=2）、lava=2、stone=3。
    fn test_registry() -> BlockRegistry {
        let toml = r#"
            [[entry]]
            id = 0
            name = "minecraft:air"

            [[entry]]
            id = 1
            name = "minecraft:water"
            level = 2

            [[entry]]
            id = 2
            name = "minecraft:flowing_lava"

            [[entry]]
            id = 3
            name = "minecraft:stone"
        "#;
        let inner = Registry::<BlockDefinition>::from_toml_str(toml).unwrap();
        BlockRegistry(inner)
    }

    #[test]
    fn constructors_are_mutually_exclusive() {
        assert!(Fluid::water(5).is_water);
        assert!(!Fluid::water(5).is_lava);
        assert!(Fluid::lava(7).is_lava);
        assert!(!Fluid::lava(7).is_water);
        assert!(Fluid::none().is_none());
        // 等级截断到 15。
        assert_eq!(Fluid::water(255).level, 15);
    }

    #[test]
    fn water_block_resolves_to_water() {
        let registry = test_registry();
        let fluid = fluid_from_block(1, &registry);
        assert!(fluid.is_water);
        assert!(!fluid.is_lava);
        // extra.level 被解析。
        assert_eq!(fluid.level, 2);
    }

    #[test]
    fn lava_block_resolves_to_lava() {
        let registry = test_registry();
        let fluid = fluid_from_block(2, &registry);
        assert!(fluid.is_lava);
        assert!(!fluid.is_water);
        assert_eq!(fluid.level, 0);
    }

    #[test]
    fn air_and_stone_resolve_to_none() {
        let registry = test_registry();
        assert!(fluid_from_block(0, &registry).is_none());
        assert!(fluid_from_block(3, &registry).is_none());
        // 未注册 id → 非流体。
        assert!(fluid_from_block(99, &registry).is_none());
    }
}
