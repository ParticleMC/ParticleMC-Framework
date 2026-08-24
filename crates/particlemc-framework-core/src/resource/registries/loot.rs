// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 战利品表注册表。
//!
//! 加载 `resources/data/loot_tables/*.toml`，每个条目为一张战利品表。
//! `pools` 是嵌套复杂结构（entries / functions / conditions），刻意以
//! `toml::Value` 原样保留、不做严格反序列化，避免嵌套结构解析失败导致
//! 整张表丢失；同时整条 entry 也会完整保留在 [`LootTable::raw`] 中，
//! 保证任何顶层附加字段（如 entry 级 `functions`）零丢失。

use std::collections::HashMap;
use std::path::Path;

use super::registry::RegistryError;

/// 单张战利品表。
///
/// `name` 是唯一键；`pools` 以原始 `toml::Value` 存盘；
/// `raw` 恒为整条 entry 的保真副本（含所有顶层字段）。
#[derive(Debug, Clone, PartialEq)]
pub struct LootTable {
    /// 战利品表名称（TOML `name`，唯一键）。
    pub name: String,
    /// 战利品表类型（TOML `type`，如 `"minecraft:block"`），缺省空串。
    pub table_type: String,
    /// 随机序列标识（TOML `random_sequence`），缺省空串。
    pub random_sequence: String,
    /// 战利品池（TOML `pools`）原始值，缺失时为 `None`。
    pub pools: Option<toml::Value>,
    /// 整条 entry 保真副本，含顶层附加字段，零丢失。
    pub raw: toml::Value,
}

/// 战利品表注册表：名称 → 战利品表。
#[derive(Default, Debug, Clone, PartialEq)]
pub struct LootTableRegistry {
    /// 以条目 `name` 为键的战利品表集合。
    pub entries: HashMap<String, LootTable>,
}

impl LootTableRegistry {
    /// 从 TOML 文本解析 `[[entry]]` 数组，构建战利品表注册表。
    ///
    /// 采用 `toml::from_str::<toml::Value>` 解析后手动提取字段，而非 serde
    /// derive 严格反序列化，避免 `pools` 这类嵌套结构解析失败拖垮整条目。
    /// 遍历 `entry` 数组，缺失 `name` 的条目直接跳过。
    ///
    /// # 错误
    /// 文本非法无法解析时返回 [`RegistryError::ParseError`]。
    pub fn from_toml_str(text: &str) -> Result<Self, RegistryError> {
        let document: toml::Value = toml::from_str(text).map_err(|_| RegistryError::ParseError)?;
        let mut entries = HashMap::new();
        if let Some(array) = document.get("entry").and_then(|value| value.as_array()) {
            for entry in array {
                // 键提取：缺失 `name` 的条目跳过。
                let name = match entry.get("name").and_then(|value| value.as_str()) {
                    Some(name) => name.to_string(),
                    None => continue,
                };
                // 缺省空串，避免 `type` / `random_sequence` 缺失时整体失败。
                let table_type = entry
                    .get("type")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string();
                let random_sequence = entry
                    .get("random_sequence")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string();
                // pools 缺失时保持 None；raw 恒为整条 entry 的保真副本。
                let pools = entry.get("pools").cloned();
                entries.insert(
                    name.clone(),
                    LootTable {
                        name,
                        table_type,
                        random_sequence,
                        pools,
                        raw: entry.clone(),
                    },
                );
            }
        }
        Ok(Self { entries })
    }

    /// 从单个 TOML 文件加载战利品表注册表（包装 [`from_toml_str`](Self::from_toml_str)）。
    ///
    /// # 错误
    /// 文件缺失或内容无法解析时返回 [`RegistryError::ParseError`]。
    pub fn from_toml_file(path: &Path) -> Result<Self, RegistryError> {
        let text = std::fs::read_to_string(path).map_err(|_| RegistryError::ParseError)?;
        Self::from_toml_str(&text)
    }

    /// 加载整个战利品表数据目录并合并所有 `.toml` 文件中的条目。
    ///
    /// 遍历目录下所有扩展名为 `toml` 的文件，逐个经
    /// [`from_toml_file`](Self::from_toml_file) 解析后合并到同一张表；
    /// 同名键以后读入的文件为准（后者覆盖前者），语义与
    /// [`GenericRegistry::load_directory`](super::generic::GenericRegistry::load_directory) 对齐。
    ///
    /// # 错误
    /// 目录不可读或任一文件解析失败时返回 [`RegistryError::ParseError`]。
    pub fn load_directory(dir: &Path) -> Result<Self, RegistryError> {
        let mut merged = HashMap::new();
        let read_dir = std::fs::read_dir(dir).map_err(|_| RegistryError::ParseError)?;
        for item in read_dir {
            let path = item.map_err(|_| RegistryError::ParseError)?.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("toml") {
                let single = Self::from_toml_file(&path)?;
                for (key, value) in single.entries {
                    merged.insert(key, value);
                }
            }
        }
        Ok(Self { entries: merged })
    }

    /// 按 name 查询战利品表。
    pub fn get(&self, name: &str) -> Option<&LootTable> {
        self.entries.get(name)
    }

    /// 战利品表数量。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否没有任何战利品表。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    /// 测试辅助：进程内自增计数器，保证并行测试下临时目录互不冲突。
    static NEXT_DIR_ID: AtomicU64 = AtomicU64::new(0);

    /// 测试辅助：临时目录守卫，析构时自动清理，避免污染系统临时目录。
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let seq = NEXT_DIR_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "loot_registry_test_{}_{}",
                std::process::id(),
                seq
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        /// 写入一个测试数据文件。
        fn write(&self, file_name: &str, text: &str) {
            std::fs::write(self.path.join(file_name), text).unwrap();
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn from_toml_str_parses_full_entry() {
        let text = r#"
[[entry]]
name = "minecraft:acacia_sapling"
type = "minecraft:block"
pools = [ { bonus_rolls = 0.0, entries = [ { type = "minecraft:item", name = "minecraft:acacia_sapling" } ], rolls = 1.0 } ]
random_sequence = "minecraft:blocks/acacia_sapling"
"#;
        let registry = LootTableRegistry::from_toml_str(text).unwrap();
        assert_eq!(registry.len(), 1);
        let table = registry.get("minecraft:acacia_sapling").unwrap();
        assert_eq!(table.name, "minecraft:acacia_sapling");
        assert_eq!(table.table_type, "minecraft:block");
        assert_eq!(table.random_sequence, "minecraft:blocks/acacia_sapling");
        // pools 作为原始 toml::Value 保留。
        assert!(table.pools.is_some());
        let pools = table.pools.as_ref().unwrap();
        assert_eq!(pools.as_array().unwrap().len(), 1);
    }

    #[test]
    fn from_toml_str_raw_keeps_top_level_extra_fields() {
        // block 表存在 entry 级 `functions` 顶层附加字段，raw 必须零丢失。
        let text = r#"
[[entry]]
name = "minecraft:gravel"
type = "minecraft:block"
functions = [ { function = "minecraft:explosion_decay" } ]
pools = [ { rolls = 1.0, entries = [ { type = "minecraft:item", name = "minecraft:gravel" } ] } ]
random_sequence = "minecraft:blocks/gravel"
"#;
        let registry = LootTableRegistry::from_toml_str(text).unwrap();
        let table = registry.get("minecraft:gravel").unwrap();
        // raw 保留整条 entry，顶层 functions 未被丢弃。
        assert_eq!(
            table
                .raw
                .get("functions")
                .and_then(|value| value.as_array())
                .map(|array| array.len()),
            Some(1)
        );
        // 结构化字段与 raw 一致。
        assert_eq!(table.table_type, "minecraft:block");
        assert_eq!(table.random_sequence, "minecraft:blocks/gravel");
        assert!(table.pools.is_some());
    }

    #[test]
    fn from_toml_str_skips_entry_without_name() {
        let text = r#"
[[entry]]
name = "minecraft:kept"
type = "minecraft:block"

[[entry]]
type = "minecraft:chest"
pools = [ { rolls = 1.0 } ]
"#;
        let registry = LootTableRegistry::from_toml_str(text).unwrap();
        assert_eq!(registry.len(), 1);
        assert!(registry.get("minecraft:kept").is_some());
        // 无 name 的条目不产生任何键。
        assert!(registry.get("type").is_none());
    }

    #[test]
    fn from_toml_str_pools_missing_is_none() {
        let text = r#"
[[entry]]
name = "minecraft:no_pools"
type = "minecraft:block"
random_sequence = "minecraft:blocks/no_pools"
"#;
        let registry = LootTableRegistry::from_toml_str(text).unwrap();
        let table = registry.get("minecraft:no_pools").unwrap();
        assert!(table.pools.is_none());
        // 缺省字段回落为空串而非报错。
        assert_eq!(table.table_type, "minecraft:block");
        assert_eq!(table.random_sequence, "minecraft:blocks/no_pools");
    }

    #[test]
    fn from_toml_str_missing_optional_fields_default_to_empty() {
        // type / random_sequence 缺失时缺省空串，pools 缺失时为 None。
        let text = r#"
[[entry]]
name = "minecraft:bare"
"#;
        let registry = LootTableRegistry::from_toml_str(text).unwrap();
        let table = registry.get("minecraft:bare").unwrap();
        assert_eq!(table.table_type, "");
        assert_eq!(table.random_sequence, "");
        assert!(table.pools.is_none());
        assert!(table.raw.get("name").is_some());
    }

    #[test]
    fn from_toml_str_bad_text_returns_parse_error() {
        let result = LootTableRegistry::from_toml_str("this is not = = valid toml @@@");
        assert!(matches!(result, Err(RegistryError::ParseError)));
    }

    #[test]
    fn from_toml_str_empty_text_yields_empty_registry() {
        assert!(LootTableRegistry::from_toml_str("").unwrap().is_empty());
    }

    #[test]
    fn from_toml_file_missing_path_returns_parse_error() {
        let missing = Path::new("does/not/exist/loot.toml");
        assert!(matches!(
            LootTableRegistry::from_toml_file(missing),
            Err(RegistryError::ParseError)
        ));
    }

    #[test]
    fn load_directory_merges_all_toml_files_with_last_key_winning() {
        let dir = TempDir::new();
        dir.write(
            "first.toml",
            r#"
[[entry]]
name = "minecraft:shared"
type = "minecraft:block"
pools = [ { rolls = 1.0 } ]
random_sequence = "minecraft:blocks/shared"

[[entry]]
name = "minecraft:only_a"
type = "minecraft:chest"
"#,
        );
        dir.write(
            "second.toml",
            r#"
[[entry]]
name = "minecraft:shared"
type = "minecraft:entity"

[[entry]]
name = "minecraft:only_b"
type = "minecraft:gameplay"
"#,
        );
        // 非 toml 扩展名文件应被忽略。
        dir.write("ignore.txt", "[[entry]]\nname = \"minecraft:ignored\"\n");

        let registry = LootTableRegistry::load_directory(dir.path()).unwrap();
        // shared / only_a / only_b 三条，ignore.txt 不参与。
        assert_eq!(registry.len(), 3);
        // 同名键：后读取的文件覆盖前者。
        assert_eq!(
            registry.get("minecraft:shared").unwrap().table_type,
            "minecraft:entity"
        );
        assert_eq!(
            registry.get("minecraft:only_a").unwrap().table_type,
            "minecraft:chest"
        );
        assert_eq!(
            registry.get("minecraft:only_b").unwrap().table_type,
            "minecraft:gameplay"
        );
        assert!(registry.get("minecraft:ignored").is_none());
    }

    #[test]
    fn load_directory_missing_dir_returns_parse_error() {
        let missing = Path::new("does/not/exist/loot_tables");
        assert!(matches!(
            LootTableRegistry::load_directory(missing),
            Err(RegistryError::ParseError)
        ));
    }

    #[test]
    fn load_directory_bad_file_returns_parse_error() {
        let dir = TempDir::new();
        dir.write("broken.toml", "this is not = = valid toml @@@");
        assert!(matches!(
            LootTableRegistry::load_directory(dir.path()),
            Err(RegistryError::ParseError)
        ));
    }
}
