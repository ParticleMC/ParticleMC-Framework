//! 实体属性组件（R8）：以 `HashMap<attr_id, AttributeInstance>` 承载实体的
//! 全部属性实例，并用 `dirty` 集合记录待同步的变更。
//!
//! 变更入口（`set_base_value` / `add_modifier` / `remove_modifier`）在应用
//! 修改的同时把 `attr_id` 标入 `dirty`，供应用侧通过 [`take_dirty`] 或
//! `AttributeInbox` 收件箱触发 `EntityAttributes`(0x81) 下发。
//! `insert` 不标脏（新建实例通常随后即被 `set_base_value` 等入口修改）。
//!
//! 见 `.specs/complete-partial-framework-capabilities/`（R8）。

use std::collections::{HashMap, HashSet};

use crate::prelude::Component;

use crate::resource::attribute::{AttributeInstance, AttributeModifier};

/// 实体属性组件。
///
/// `instances` 以属性注册表 id 为键；`dirty` 为等待网络同步的 id 集合。
#[derive(Component, Clone, Debug, Default)]
#[component(storage = "sparse")]
pub struct Attributes {
    /// 属性实例表（键为注册表 id）。
    pub instances: HashMap<u32, AttributeInstance>,
    /// 待同步的属性 id 集合。
    pub dirty: HashSet<u32>,
}

impl Attributes {
    /// 读取属性实例；未挂载时返回 `None`。
    pub fn get(&self, attr_id: u32) -> Option<&AttributeInstance> {
        self.instances.get(&attr_id)
    }

    /// 插入（或覆盖）属性实例。不标脏——新建实例需经
    /// [`set_base_value`](Self::set_base_value) / [`add_modifier`](Self::add_modifier)
    /// 等变更入口才进入同步队列。
    pub fn insert(&mut self, instance: AttributeInstance) {
        let id = instance.attribute.id;
        self.instances.insert(id, instance);
    }

    /// 设置属性基值并标脏；属性未挂载时忽略（不 panic）。
    pub fn set_base_value(&mut self, attr_id: u32, base_value: f64) {
        if let Some(instance) = self.instances.get_mut(&attr_id) {
            instance.set_base_value(base_value);
            self.dirty.insert(attr_id);
        }
    }

    /// 添加修饰器并标脏；属性未挂载时忽略（不 panic）。
    pub fn add_modifier(&mut self, attr_id: u32, modifier: AttributeModifier) {
        if let Some(instance) = self.instances.get_mut(&attr_id) {
            instance.add_modifier(modifier);
            self.dirty.insert(attr_id);
        }
    }

    /// 移除修饰器并标脏；返回被移除者（属性或修饰器不存在时返回 `None`）。
    pub fn remove_modifier(
        &mut self,
        attr_id: u32,
        modifier_id: &str,
    ) -> Option<AttributeModifier> {
        let removed = self
            .instances
            .get_mut(&attr_id)?
            .remove_modifier(modifier_id);
        if removed.is_some() {
            self.dirty.insert(attr_id);
        }
        removed
    }

    /// 取出并清空待同步集合（升序返回，保证确定性）。
    pub fn take_dirty(&mut self) -> Vec<u32> {
        let mut ids: Vec<u32> = std::mem::take(&mut self.dirty).into_iter().collect();
        ids.sort_unstable();
        ids
    }

    /// 计算属性最终值（含修饰器与 min/max 裁剪）；属性未挂载时返回 `None`。
    pub fn value(&self, attr_id: u32) -> Option<f64> {
        self.instances.get(&attr_id).map(AttributeInstance::value)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::resource::attribute::{Attribute, AttributeOperation};

    fn attr(id: u32, name: &str, default: f64) -> Attribute {
        Attribute {
            name: name.to_string(),
            id,
            default_value: default,
            min_value: 0.0,
            max_value: 100.0,
            client_sync: true,
        }
    }

    #[test]
    fn insert_get_and_value_roundtrip() {
        let mut attributes = Attributes::default();
        assert!(attributes.get(19).is_none());
        assert_eq!(attributes.value(19), None);
        attributes.insert(AttributeInstance::new(attr(
            19,
            "minecraft:max_health",
            20.0,
        )));
        assert_eq!(attributes.get(19).unwrap().get_base_value(), 20.0);
        assert_eq!(attributes.value(19), Some(20.0));
        // insert 不标脏。
        assert!(attributes.dirty.is_empty());
    }

    #[test]
    fn set_base_value_marks_dirty_and_changes_value() {
        let mut attributes = Attributes::default();
        attributes.insert(AttributeInstance::new(attr(
            19,
            "minecraft:max_health",
            20.0,
        )));
        attributes.set_base_value(19, 30.0);
        assert!(attributes.dirty.contains(&19));
        assert_eq!(attributes.value(19), Some(30.0));
        // 未挂载的属性 set_base_value 被忽略。
        attributes.set_base_value(99, 5.0);
        assert!(!attributes.dirty.contains(&99));
        assert_eq!(attributes.value(99), None);
    }

    #[test]
    fn add_remove_modifier_affects_value_and_dirty() {
        let mut attributes = Attributes::default();
        attributes.insert(AttributeInstance::new(attr(
            19,
            "minecraft:max_health",
            10.0,
        )));
        attributes.add_modifier(
            19,
            AttributeModifier {
                id: "m:add".into(),
                amount: 5.0,
                operation: AttributeOperation::Add,
            },
        );
        assert!(attributes.dirty.contains(&19));
        assert_eq!(attributes.value(19), Some(15.0));

        let removed = attributes.remove_modifier(19, "m:add");
        assert_eq!(removed.unwrap().amount, 5.0);
        assert!(attributes.dirty.contains(&19));
        assert_eq!(attributes.value(19), Some(10.0));
        // 移除不存在修饰器：不标脏、返回 None。
        let dirty_before = attributes.take_dirty();
        attributes.remove_modifier(19, "m:missing");
        assert!(attributes.dirty.is_empty());
        // 未挂载属性移除：返回 None 且不标脏。
        assert!(attributes.remove_modifier(99, "x").is_none());
        assert!(attributes.dirty.is_empty());
        assert_eq!(dirty_before, vec![19]);
    }

    #[test]
    fn take_dirty_drains_and_clears() {
        let mut attributes = Attributes::default();
        attributes.insert(AttributeInstance::new(attr(
            19,
            "minecraft:max_health",
            20.0,
        )));
        attributes.insert(AttributeInstance::new(attr(
            22,
            "minecraft:movement_speed",
            0.7,
        )));
        attributes.set_base_value(19, 25.0);
        attributes.set_base_value(22, 0.8);
        // 第一次取走：升序两个 id。
        assert_eq!(attributes.take_dirty(), vec![19, 22]);
        assert!(attributes.dirty.is_empty());
        // 再次取走为空。
        assert!(attributes.take_dirty().is_empty());
        // 再次变更仍能重新标脏。
        attributes.set_base_value(19, 26.0);
        assert_eq!(attributes.take_dirty(), vec![19]);
    }
}
