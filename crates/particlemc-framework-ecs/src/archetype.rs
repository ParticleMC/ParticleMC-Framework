// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 静态 Archetype 定义：实体类型到固定组件集合的映射。
//!
//! 变更标识符：`implement-custom-ecs`
//!
//! 实体创建直接指定 `ArchetypeId`（R2.3/RM2），运行时禁止动态创建新
//! Archetype。`ArchetypeDef` 是宏侧（`#[derive(Archetype)]` +
//! `register_archetypes!`，T2）与运行时存储侧（T3/T4）之间的数据契约：
//! 字段形态必须与宏生成的 `archetype_def()` 构造代码完全一致。
//!
//! 表聚合 `ARCHETYPES` 由 `register_archetypes!` 宏生成（AI Amendment A3：
//! 组件 ID 惰性分配导致无法 const 求值，故采用 `LazyLock<&'static [ArchetypeDef]>`
//! 表达静态表语义）。

use std::any::TypeId;

use crate::component::ComponentId;
use crate::entity::EntityTypeId;

/// Archetype 标识：取值即 `register_archetypes!` 表中下标（0 起始递增，IC-3）。
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct ArchetypeId(pub u16);

/// 静态 Archetype 的运行时定义（与 T2 宏生成的 `archetype_def()` 字段对齐）。
///
/// - `component_ids`：该 Archetype 的固定组件集合（SoA 组件）；Sparse 组件
///   不在此列（R3.3，任意增删）。
/// - `entity_kind`：该 Archetype 所有实体共享的实体类型 ID（R1.4）。
/// - `component_types`：与 `component_ids` 同序的类型擦除镜像，供运行时在
///   无具体类型信息处做类型比对/分发。
///
/// 已 derive `Clone`/`Copy`：引用字段均为 `'static`，整体可任意复制而无
/// 生命周期负担；`register_archetype` 仍接受 `&'static` 入参并内部复制，
/// 运行时亦可构造合成空 Archetype 的拥有副本（T11 迁移）。
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct ArchetypeDef {
    /// 表下标即 ID（宏按注册顺序分配）。
    pub id: ArchetypeId,
    /// 声明该 Archetype 的类型名（`stringify!` 产物）。
    pub name: &'static str,
    /// 固定组件集合（SoA 组件 ID 列表，与 `component_types` 同序）。
    pub component_ids: &'static [ComponentId],
    /// 该 Archetype 的实体类型 ID。
    pub entity_kind: EntityTypeId,
    /// 组件类型擦除镜像（T4 Query 匹配与 T3 列类型校验用）。
    pub component_types: &'static [TypeId],
}

impl ArchetypeDef {
    /// 组件 ID 是否属于该 Archetype 的固定组件集合。
    pub fn has_component(&self, c: ComponentId) -> bool {
        self.component_ids.contains(&c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用静态 ArchetypeDef：`&[]` 空切片具备 `'static` 提升。
    static TEST_DEF: ArchetypeDef = ArchetypeDef {
        id: ArchetypeId(0),
        name: "TestArchetype",
        component_ids: &[ComponentId(1), ComponentId(2)],
        entity_kind: EntityTypeId(7),
        component_types: &[],
    };

    #[test]
    fn has_component_hit() {
        assert!(TEST_DEF.has_component(ComponentId(1)));
        assert!(TEST_DEF.has_component(ComponentId(2)));
    }

    #[test]
    fn has_component_miss() {
        assert!(!TEST_DEF.has_component(ComponentId(0)));
        assert!(!TEST_DEF.has_component(ComponentId(3)));
    }

    #[test]
    fn def_fields_match_declaration() {
        // 字段契约：T2 宏 `archetype_def()` 按此形态构造，字段名/类型不得漂移
        assert_eq!(TEST_DEF.id, ArchetypeId(0));
        assert_eq!(TEST_DEF.name, "TestArchetype");
        assert_eq!(TEST_DEF.component_ids.len(), 2);
        assert_eq!(TEST_DEF.entity_kind, EntityTypeId(7));
        assert!(TEST_DEF.component_types.is_empty());
    }
}
