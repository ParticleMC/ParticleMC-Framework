//! 伤害值类型（T4 最小版 → T7 伤害体系扩展）。
//!
//! 本模块承载 [`Living::hurt`] 委托 [`Health::damage`] 所需的 [`Damage`] 值。
//! T7 在 T4 的 `amount` 之上追加伤害类型（[`DamageType`] 注册表条目）与伤害来源
//! 语义（实体 / 位置 / 类型），扩展保持 `amount` 字段与 T4 [`Living::hurt`]
//! 契约兼容。
//!
//! 变更标识符：`complete-missing-subsystems`（T4/T7）。

use crate::resource::DamageType;

/// 一次伤害：伤害量（`f32`，单位与 [`Health`] 一致）、类型与来源。
#[derive(Debug, Clone, PartialEq)]
pub struct Damage {
    /// 伤害量。
    pub amount: f32,
    /// 伤害类型（`None` 表示未指定，调用方按通用伤害处理）。
    pub damage_type: Option<DamageType>,
    /// 伤害来源。
    pub source: DamageSource,
}

impl Damage {
    /// 以伤害量构造便捷入口（来源 `Unknown`、无伤害类型）。
    ///
    /// 需要携带类型 / 来源时使用结构体字面量或直接修改对应字段。
    pub fn new(amount: f32) -> Self {
        Self {
            amount,
            damage_type: None,
            source: DamageSource::Unknown,
        }
    }
}

/// 伤害来源。
///
/// `Entity(u32)` 以 ECS 实体 id 标记攻击者；`Positional { x, y, z }` 为带坐标的
/// 非实体环境来源（岩浆、坠落、窒息等）；`Type(String)` 为以注册表名称标记的
/// 来源（自定义伤害类型）；`Unknown` 为未知来源。
#[derive(Debug, Clone, PartialEq)]
pub enum DamageSource {
    /// 来自某实体（携带其 ECS 实体 id）。
    Entity(u32),
    /// 带坐标的非实体环境来源。
    Positional { x: f64, y: f64, z: f64 },
    /// 以伤害类型名称标记的来源。
    Type(String),
    /// 未知来源。
    Unknown,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn damage_constructs_with_amount() {
        let damage = Damage::new(5.0);
        assert_eq!(damage.amount, 5.0);
    }

    #[test]
    fn damage_new_defaults_to_unknown_source_without_type() {
        let damage = Damage::new(3.0);
        assert_eq!(damage.source, DamageSource::Unknown);
        assert!(damage.damage_type.is_none());
    }

    #[test]
    fn damage_fields_can_be_set_explicitly() {
        let damage = Damage {
            amount: 4.0,
            damage_type: None,
            source: DamageSource::Positional {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
        };
        assert_eq!(damage.amount, 4.0);
        assert_eq!(
            damage.source,
            DamageSource::Positional {
                x: 1.0,
                y: 2.0,
                z: 3.0
            }
        );
    }

    #[test]
    fn damage_source_variants() {
        assert_eq!(DamageSource::Entity(42), DamageSource::Entity(42));
        assert_eq!(
            DamageSource::Positional {
                x: 0.0,
                y: 1.0,
                z: 2.0
            },
            DamageSource::Positional {
                x: 0.0,
                y: 1.0,
                z: 2.0
            }
        );
        assert_eq!(
            DamageSource::Type("minecraft:fall".to_string()),
            DamageSource::Type("minecraft:fall".to_string())
        );
        assert_eq!(DamageSource::Unknown, DamageSource::Unknown);
        assert_ne!(DamageSource::Entity(1), DamageSource::Entity(2));
        assert_ne!(
            DamageSource::Unknown,
            DamageSource::Type("minecraft:fall".to_string())
        );
    }
}
