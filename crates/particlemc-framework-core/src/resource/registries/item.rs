// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 物品注册表：以 `Resource` 形式承载 `Registry<ItemDefinition>`。

use std::path::Path;

use super::registry::{ItemDefinition, Registry, RegistryError};

/// 物品注册表（具名 `Resource`）。
#[derive(Default, Debug, Clone)]
pub struct ItemRegistry(pub Registry<ItemDefinition>);

impl ItemRegistry {
    /// 从 TOML 文件加载物品注册表。
    pub fn from_toml_file(path: &Path) -> Result<Self, RegistryError> {
        Ok(Self(Registry::from_toml_file(path)?))
    }

    /// 覆盖或注册一个物品条目：name 已存在时保留原 id、替换 value；
    /// 不存在时等价注册。转发到 [`Registry::override_value`]。
    pub fn override_value(
        &mut self,
        name: &str,
        value: ItemDefinition,
    ) -> Result<(), RegistryError> {
        self.0.override_value(name, value)
    }

    /// 注册或替换一个物品条目，返回其 id。转发到 [`Registry::register_or_replace`]。
    pub fn register_or_replace(
        &mut self,
        name: impl Into<String>,
        value: ItemDefinition,
    ) -> Result<u32, RegistryError> {
        self.0.register_or_replace(name, value)
    }
}
