//! 属性框架（R8）：`Attribute` 值类型、`AttributeInstance` 叠加实例与
//! [`AttributeRegistry`] 注册表。
//!
//! 属性清单（name / 注册表 id / default / min / max / client_sync）由
//! `resources/data/attributes.toml` 驱动，`AttributeImpl.REGISTRY` 的序位为
//! id 权威来源（35 项，id 0..=34，与 Java `Attributes.java` 常量清单一致）。
//! 叠加规则对齐 Java `AttributeInstance.computeValue`：先累加 ADD，再以
//! 累加后的 base 计算 ADD_MULTIPLIED_BASE，最后连乘 MULTIPLIED_TOTAL，
//! 最终裁剪到 `[min_value, max_value]`。
//!
//! 见 `.specs/complete-partial-framework-capabilities/`（R8）。

use std::path::Path;

use serde::Deserialize;

use crate::resource::registries::{Registry, RegistryEntry, RegistryError};

/// 属性定义（注册表条目）。
///
/// 与 Java `Attribute` 接口对齐：以注册表驱动的方式承载 `id`（线格式注册表
/// 序号）、默认值、min/max 边界与是否下发客户端（`client_sync`）。
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Attribute {
    /// 命名空间名称，如 `minecraft:max_health`。
    pub name: String,
    /// 注册表 id（1.21.11 线格式 `EntityAttributes` 使用该序号）。
    pub id: u32,
    /// 默认基值（构造 [`AttributeInstance`] 时采用）。
    pub default_value: f64,
    /// 最小值（`value()` 裁剪下限）。
    pub min_value: f64,
    /// 最大值（`value()` 裁剪上限）。
    pub max_value: f64,
    /// 是否随 `EntityAttributes`(0x81) 下发客户端。
    pub client_sync: bool,
}

impl RegistryEntry for Attribute {
    fn entry_name(&self) -> &str {
        &self.name
    }

    fn entry_id(&self) -> Option<u32> {
        Some(self.id)
    }
}

/// 属性修饰器操作类型（线格式 VarInt 序位）。
///
/// 叠加顺序固定：先全部 `Add`，再全部 `AddMultipliedBase`，最后全部
/// `MultipliedTotal`（与 Java `AttributeOperation` 一致）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttributeOperation {
    /// 直接加值（线格式 0）。
    Add = 0,
    /// 以「累加 ADD 后的基值」乘加（线格式 1）。
    AddMultipliedBase = 1,
    /// 对最终结果整体乘算 `1.0 + amount`（线格式 2）。
    MultipliedTotal = 2,
}

impl AttributeOperation {
    /// 返回线格式 VarInt 序位（`0`/`1`/`2`）。
    ///
    /// 使用显式 `match` 而非 `as` 转换，遵守「禁止 `as` 缩窄」章程。
    pub fn wire_value(self) -> i32 {
        match self {
            AttributeOperation::Add => 0,
            AttributeOperation::AddMultipliedBase => 1,
            AttributeOperation::MultipliedTotal => 2,
        }
    }
}

/// 属性修饰器：以 `id` 标识，携带 `amount` 与操作类型。
///
/// 同一属性实例内修饰器按 `id` 去重（重复添加时替换旧值）。
#[derive(Clone, Debug, PartialEq)]
pub struct AttributeModifier {
    /// 修饰器 id（命名空间键，如 `minecraft:some_modifier`）。
    pub id: String,
    /// 修饰量。
    pub amount: f64,
    /// 操作类型。
    pub operation: AttributeOperation,
}

/// 单个属性的运行时实例：基值 + 修饰器集合。
///
/// 计算语义对齐 Java `AttributeInstance.computeValue`：
/// 1. `value = base_value`；
/// 2. 全部 `Add`：`value += amount`；
/// 3. 全部 `AddMultipliedBase`：`value += base * amount`（`base` 为步骤 2
///    累加后的值，Java 以该值乘加）；
/// 4. 全部 `MultipliedTotal`：`value *= (1.0 + amount)`；
/// 5. 裁剪到 `[min_value, max_value]`（仅当 min ≤ max 时）。
#[derive(Clone, Debug, PartialEq)]
pub struct AttributeInstance {
    /// 关联的属性定义。
    pub attribute: Attribute,
    /// 基值（不含修饰器）。
    base_value: f64,
    /// 修饰器列表（按 `id` 去重）。
    modifiers: Vec<AttributeModifier>,
}

impl AttributeInstance {
    /// 以属性的默认值作为基值构造实例。
    pub fn new(attribute: Attribute) -> Self {
        let base_value = attribute.default_value;
        Self {
            attribute,
            base_value,
            modifiers: Vec::new(),
        }
    }

    /// 读取基值。
    pub fn get_base_value(&self) -> f64 {
        self.base_value
    }

    /// 设置基值（不裁剪；裁剪发生在 `value()` 计算时）。
    pub fn set_base_value(&mut self, base_value: f64) {
        self.base_value = base_value;
    }

    /// 添加修饰器；同 `id` 已存在时替换旧值并返回旧值，否则返回 `None`。
    pub fn add_modifier(&mut self, modifier: AttributeModifier) -> Option<AttributeModifier> {
        if let Some(existing) = self.modifiers.iter_mut().find(|m| m.id == modifier.id) {
            Some(std::mem::replace(existing, modifier))
        } else {
            self.modifiers.push(modifier);
            None
        }
    }

    /// 按 `id` 移除修饰器；存在时返回被移除者，否则返回 `None`。
    pub fn remove_modifier(&mut self, id: &str) -> Option<AttributeModifier> {
        let index = self.modifiers.iter().position(|m| m.id == id)?;
        Some(self.modifiers.remove(index))
    }

    /// 当前全部修饰器（只读切片）。
    pub fn modifiers(&self) -> &[AttributeModifier] {
        &self.modifiers
    }

    /// 应用修饰器后的最终值（含 min/max 裁剪），对齐 Java `computeValue`。
    pub fn value(&self) -> f64 {
        // 1) 基值 + 全部 ADD 修饰器（Java 中该局部变量即为「base」）。
        let mut base = self.base_value;
        for m in &self.modifiers {
            if m.operation == AttributeOperation::Add {
                base += m.amount;
            }
        }
        // 2) ADD_MULTIPLIED_BASE 以「累加后的 base」乘加。
        let mut result = base;
        for m in &self.modifiers {
            if m.operation == AttributeOperation::AddMultipliedBase {
                result += base * m.amount;
            }
        }
        // 3) MULTIPLIED_TOTAL 整体连乘。
        for m in &self.modifiers {
            if m.operation == AttributeOperation::MultipliedTotal {
                result *= 1.0 + m.amount;
            }
        }
        // 4) 裁剪到 [min, max]（仅当边界合法时；Java 语义为先累加再裁剪）。
        let min = self.attribute.min_value;
        let max = self.attribute.max_value;
        if min <= max {
            result.clamp(min, max)
        } else {
            result
        }
    }
}

/// 属性注册表（具名 `Resource`）。
///
/// 内部委托 [`Registry<Attribute>`] 承担「name ⇄ id」映射；条目由
/// `resources/data/attributes.toml` 加载（`[[entry]]` 数组，id 以 Java
/// `AttributeImpl.REGISTRY` 清单序位为准）。
#[derive(Default, Debug, Clone)]
pub struct AttributeRegistry {
    /// 底层注册表。
    inner: Registry<Attribute>,
}

impl AttributeRegistry {
    /// 从 TOML 文件加载属性注册表（路径不存在或解析失败返回
    /// [`RegistryError`]，由调用方决定回退为空表）。
    pub fn from_toml_file(path: &Path) -> Result<Self, RegistryError> {
        Ok(Self {
            inner: Registry::<Attribute>::from_toml_file(path)?,
        })
    }

    /// 从 TOML 文本加载属性注册表（主要供单元测试使用）。
    pub fn from_toml_str(text: &str) -> Result<Self, RegistryError> {
        Ok(Self {
            inner: Registry::<Attribute>::from_toml_str(text)?,
        })
    }

    /// 按注册表 id 查询属性。
    pub fn by_id(&self, id: u32) -> Option<&Attribute> {
        self.inner.get(id)
    }

    /// 按命名空间名称查询属性。
    pub fn by_name(&self, name: &str) -> Option<&Attribute> {
        self.inner.get_id(name).and_then(|id| self.inner.get(id))
    }

    /// 遍历全部属性（顺序不定）。
    pub fn all(&self) -> impl Iterator<Item = &Attribute> {
        self.inner.values()
    }

    /// 已注册属性数量。
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// 注册表是否为空。
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// 构造一个便于测试的属性定义（默认值 10、边界 [0, 100]）。
    fn attr(id: u32, name: &str, client_sync: bool) -> Attribute {
        Attribute {
            name: name.to_string(),
            id,
            default_value: 10.0,
            min_value: 0.0,
            max_value: 100.0,
            client_sync,
        }
    }

    #[test]
    fn new_uses_attribute_default_as_base() {
        let mut a = attr(19, "minecraft:max_health", true);
        a.default_value = 20.0;
        let instance = AttributeInstance::new(a);
        assert_eq!(instance.get_base_value(), 20.0);
        assert_eq!(instance.value(), 20.0);
        assert!(instance.modifiers().is_empty());
    }

    #[test]
    fn value_stacks_add_then_multiplied_total() {
        // 规格场景：base=10 + ADD 5 + MULTIPLIED_TOTAL 0.1 → (10+5)*1.1 = 16.5。
        let mut instance = AttributeInstance::new(attr(19, "minecraft:max_health", true));
        instance.add_modifier(AttributeModifier {
            id: "minecraft:add".into(),
            amount: 5.0,
            operation: AttributeOperation::Add,
        });
        instance.add_modifier(AttributeModifier {
            id: "minecraft:total".into(),
            amount: 0.1,
            operation: AttributeOperation::MultipliedTotal,
        });
        assert_eq!(instance.value(), 16.5);
    }

    #[test]
    fn value_uses_post_add_base_for_multiplied_base() {
        // 对齐 Java：ADD_MULTIPLIED_BASE 以「累加 ADD 后的基值」乘加。
        // base=10 + ADD 2 → base=12；AMB 0.5 → 12 + 12*0.5 = 18。
        let mut instance = AttributeInstance::new(attr(1, "minecraft:armor_toughness", true));
        instance.add_modifier(AttributeModifier {
            id: "m:add".into(),
            amount: 2.0,
            operation: AttributeOperation::Add,
        });
        instance.add_modifier(AttributeModifier {
            id: "m:amb".into(),
            amount: 0.5,
            operation: AttributeOperation::AddMultipliedBase,
        });
        assert_eq!(instance.value(), 18.0);
    }

    #[test]
    fn value_clamps_to_min_and_max() {
        // 上限：base=10 + ADD 100 → 110，裁剪到 max=100。
        let mut instance = AttributeInstance::new(attr(19, "minecraft:max_health", true));
        instance.add_modifier(AttributeModifier {
            id: "m:add".into(),
            amount: 100.0,
            operation: AttributeOperation::Add,
        });
        assert_eq!(instance.value(), 100.0);
        // 下限：base=10 + ADD -50 → -40，裁剪到 min=0。
        let mut instance = AttributeInstance::new(attr(19, "minecraft:max_health", true));
        instance.add_modifier(AttributeModifier {
            id: "m:add".into(),
            amount: -50.0,
            operation: AttributeOperation::Add,
        });
        assert_eq!(instance.value(), 0.0);
        // min > max（异常数据）时不裁剪。
        let mut bad = attr(0, "minecraft:armor", true);
        bad.min_value = 10.0;
        bad.max_value = 5.0;
        let instance = AttributeInstance::new(bad);
        assert_eq!(instance.value(), 10.0);
    }

    #[test]
    fn modifier_add_replaces_by_id_and_remove_restores_value() {
        let mut instance = AttributeInstance::new(attr(19, "minecraft:max_health", true));
        // 添加 ADD +5。
        let replaced = instance.add_modifier(AttributeModifier {
            id: "m:add".into(),
            amount: 5.0,
            operation: AttributeOperation::Add,
        });
        assert!(replaced.is_none());
        assert_eq!(instance.value(), 15.0);
        // 同 id 替换为 +10 → 返回旧修饰器，value 变为 20。
        let replaced = instance.add_modifier(AttributeModifier {
            id: "m:add".into(),
            amount: 10.0,
            operation: AttributeOperation::Add,
        });
        assert_eq!(replaced.as_ref().map(|m| m.amount), Some(5.0));
        assert_eq!(instance.value(), 20.0);
        // 移除 → 回到基值 10。
        let removed = instance.remove_modifier("m:add");
        assert_eq!(removed.as_ref().map(|m| m.amount), Some(10.0));
        assert_eq!(instance.value(), 10.0);
        // 移除不存在的 id → None。
        assert!(instance.remove_modifier("m:missing").is_none());
    }

    #[test]
    fn registry_loads_from_toml_and_queries() {
        let toml = r#"
            [[entry]]
            name = "minecraft:max_health"
            id = 19
            default_value = 20.0
            min_value = 1.0
            max_value = 1024.0
            client_sync = true

            [[entry]]
            name = "minecraft:movement_speed"
            id = 22
            default_value = 0.7
            min_value = 0.0
            max_value = 1024.0
            client_sync = true

            [[entry]]
            name = "minecraft:follow_range"
            id = 13
            default_value = 32.0
            min_value = 0.0
            max_value = 2048.0
            client_sync = false
        "#;
        let registry = AttributeRegistry::from_toml_str(toml).unwrap();
        assert_eq!(registry.len(), 3);
        let health = registry.by_id(19).unwrap();
        assert_eq!(health.name, "minecraft:max_health");
        assert_eq!(health.default_value, 20.0);
        assert!(health.client_sync);
        assert_eq!(registry.by_name("minecraft:movement_speed").unwrap().id, 22);
        assert!(registry.by_id(99).is_none());
        assert!(registry.by_name("minecraft:missing").is_none());
        let sync_flags: Vec<bool> = registry.all().map(|a| a.client_sync).collect();
        assert_eq!(sync_flags.len(), 3);
        assert!(
            !registry
                .by_name("minecraft:follow_range")
                .unwrap()
                .client_sync
        );
    }

    #[test]
    fn registry_loads_real_data_file() {
        // 真实注册数据：35 项，id 与 Java Attributes.java 序位一致。
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/data/attributes.toml");
        let registry = AttributeRegistry::from_toml_file(&path).unwrap();
        assert_eq!(registry.len(), 35);
        assert_eq!(registry.by_name("minecraft:max_health").unwrap().id, 19);
        assert_eq!(registry.by_name("minecraft:movement_speed").unwrap().id, 22);
        assert_eq!(registry.by_name("minecraft:armor").unwrap().id, 0);
        assert_eq!(
            registry.by_id(34).unwrap().name,
            "minecraft:waypoint_receive_range"
        );
    }

    #[test]
    fn wire_value_matches_ordinal() {
        assert_eq!(AttributeOperation::Add.wire_value(), 0);
        assert_eq!(AttributeOperation::AddMultipliedBase.wire_value(), 1);
        assert_eq!(AttributeOperation::MultipliedTotal.wire_value(), 2);
    }
}
