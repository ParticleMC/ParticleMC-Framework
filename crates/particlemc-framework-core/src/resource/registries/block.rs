// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 方块注册表：以 `Resource` 形式承载 `Registry<BlockDefinition>`。
//!
//! 启动时由 [`crate::plugin::McServerPlugin`] 调用 [`BlockRegistry::from_json_file`]
//! 从 `resources/data/blocks.json` 填充，框架其余部分通过 `Res<BlockRegistry>` 读取。

use std::path::Path;

use super::registry::{BlockDefinition, Registry, RegistryError};

/// 方块注册表（具名 `Resource`）。
#[derive(Default, Debug, Clone)]
pub struct BlockRegistry(pub Registry<BlockDefinition>);

impl BlockRegistry {
    /// 从 TOML 文件加载方块注册表。
    ///
    /// # 错误
    /// 文件缺失或解析失败返回 [`RegistryError`]，不 panic。
    pub fn from_toml_file(path: &Path) -> Result<Self, RegistryError> {
        Ok(Self(Registry::from_toml_file(path)?))
    }

    /// 从 JSON 文件加载方块注册表。
    ///
    /// # 错误
    /// 文件缺失或解析失败返回 [`RegistryError`]，不 panic。
    pub fn from_json_file(path: &Path) -> Result<Self, RegistryError> {
        Ok(Self(Registry::from_json_file(path)?))
    }
}
