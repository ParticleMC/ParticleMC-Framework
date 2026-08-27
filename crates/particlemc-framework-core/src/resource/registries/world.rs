// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 世界类具名注册表集合。
//!
//! 生物群系、维度类型、流体、粒子、音效事件、附魔、药水效果这七类
//! 注册表结构相近（均只有 `name` + 透传字段，部分为无 id 数据源），统一使用
//! [`GenericDefinition`] 承载，避免为每类各写一套字段结构。伤害类型由
//! [`crate::resource::damage_type::DamageTypeRegistry`]（T7 语义注册表）接管。

use std::path::Path;

use super::registry::{GenericDefinition, Registry, RegistryError};

macro_rules! define_named_registry {
    ($name:ident, $doc:expr) => {
        #[doc = $doc]
        #[derive(Default, Debug, Clone)]
        pub struct $name(pub Registry<GenericDefinition>);

        impl $name {
            /// 从 TOML 文件加载该注册表。
            pub fn from_toml_file(path: &Path) -> Result<Self, RegistryError> {
                Ok(Self(Registry::from_toml_file(path)?))
            }

            /// 从 JSON 文件加载该注册表。
            pub fn from_json_file(path: &Path) -> Result<Self, RegistryError> {
                Ok(Self(Registry::from_json_file(path)?))
            }
        }
    };
}

define_named_registry!(BiomeRegistry, "生物群系注册表（具名 `Resource`）。");
define_named_registry!(DimensionTypeRegistry, "维度类型注册表（具名 `Resource`）。");
define_named_registry!(FluidRegistry, "流体注册表（具名 `Resource`）。");
define_named_registry!(ParticleRegistry, "粒子注册表（具名 `Resource`）。");
define_named_registry!(SoundEventRegistry, "音效事件注册表（具名 `Resource`）。");
define_named_registry!(EnchantmentRegistry, "附魔注册表（具名 `Resource`）。");
define_named_registry!(PotionEffectRegistry, "药水效果注册表（具名 `Resource`）。");
