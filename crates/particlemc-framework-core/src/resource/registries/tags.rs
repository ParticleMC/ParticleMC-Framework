//! 标签注册表。
//!
//! 加载 `resources/data/tags/*.toml`，每个标签条目由 `name` 与 `values`
//! （命名空间字符串列表，可含 `#` 前缀的标签引用）组成。

use std::collections::HashMap;
use std::path::Path;

use super::registry::RegistryError;

/// 标签注册表：tag 名称 → 其包含的命名空间值列表。
#[derive(Default, Debug, Clone)]
pub struct TagRegistry {
    /// 以标签名（如 `minecraft:azalea_root_replaceable`）为键的标签集合。
    pub tags: HashMap<String, Vec<String>>,
}

impl TagRegistry {
    /// 从 TOML 文本解析 `[[entry]]` 数组，构建标签注册表。
    ///
    /// # 错误
    /// 文本非法或某条目缺少 `name` 时返回 [`RegistryError`]。
    pub fn from_toml_str(text: &str) -> Result<Self, RegistryError> {
        let document: toml::Value = toml::from_str(text).map_err(|_| RegistryError::ParseError)?;
        let mut tags = HashMap::new();
        if let Some(array) = document.get("entry").and_then(|value| value.as_array()) {
            for entry in array {
                let name = entry
                    .get("name")
                    .and_then(|value| value.as_str())
                    .ok_or(RegistryError::ParseError)?
                    .to_string();
                let values = entry
                    .get("values")
                    .and_then(|value| value.as_array())
                    .map(|array| {
                        array
                            .iter()
                            .filter_map(|item| item.as_str().map(|text| text.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                tags.insert(name, values);
            }
        }
        Ok(Self { tags })
    }

    /// 从单个标签 TOML 文件加载。
    ///
    /// # 错误
    /// 文件缺失、解析失败或某条目缺少 `name` 时返回 [`RegistryError`]。
    pub fn from_toml_file(path: &Path) -> Result<Self, RegistryError> {
        let text = std::fs::read_to_string(path).map_err(|_| RegistryError::ParseError)?;
        Self::from_toml_str(&text)
    }

    /// 加载整个 `tags` 目录并合并所有 `.toml` 文件中的标签。
    ///
    /// # 错误
    /// 目录不可读时返回 [`RegistryError::ParseError`]。
    pub fn load_directory(dir: &Path) -> Result<Self, RegistryError> {
        let mut merged = HashMap::new();
        let read_dir = std::fs::read_dir(dir).map_err(|_| RegistryError::ParseError)?;
        for item in read_dir {
            let path = item.map_err(|_| RegistryError::ParseError)?.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("toml") {
                let single = Self::from_toml_file(&path)?;
                for (key, value) in single.tags {
                    merged.insert(key, value);
                }
            }
        }
        Ok(Self { tags: merged })
    }

    /// 查询某个标签的值列表。
    pub fn get(&self, name: &str) -> Option<&Vec<String>> {
        self.tags.get(name)
    }

    /// 标签数量。
    pub fn len(&self) -> usize {
        self.tags.len()
    }

    /// 是否没有任何标签。
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
    }
}
