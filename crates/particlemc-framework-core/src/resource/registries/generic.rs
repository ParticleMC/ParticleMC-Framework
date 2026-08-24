//! 通用变体类注册表。
//!
//! 承接绘画变体、横幅图案、修剪材料等「无专属结构」的注册数据，
//! 以 `HashMap<String, toml::Value>` 原样存盘，保证任何字段都不会在加载时丢失。

use std::collections::HashMap;
use std::path::Path;

use super::registry::RegistryError;

/// 通用注册表：name → 原始 TOML 值（整条 `[[entry]]`）。
#[derive(Default, Debug, Clone)]
pub struct GenericRegistry {
    /// 以条目 `name` 为键、整条 TOML 表为值的存盘。
    pub entries: HashMap<String, toml::Value>,
    /// 自增 id 分配器（供 [`register_or_replace`](Self::register_or_replace) 使用）。
    next_id: u32,
    /// name → 已分配 id 记录：覆盖已登记条目时据此保留原 id、不推进自增。
    ids: HashMap<String, u32>,
}

impl GenericRegistry {
    /// 构造一个空注册表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 从 TOML 文本解析 `[[entry]]` 数组，构建通用注册表。
    ///
    /// # 错误
    /// 文本非法或结构不符返回 [`RegistryError::ParseError`]。
    pub fn from_toml_str(text: &str) -> Result<Self, RegistryError> {
        let document: toml::Value = toml::from_str(text).map_err(|_| RegistryError::ParseError)?;
        let mut entries = HashMap::new();
        if let Some(array) = document.get("entry").and_then(|value| value.as_array()) {
            for entry in array {
                // 键提取：优先使用 `name` 字符串；无 `name` 时回退 `id` 字段
                // （整数值转十进制字符串，如 `0` → `"0"`）；两者皆无才跳过该条。
                let key = entry
                    .get("name")
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
                    .or_else(|| {
                        entry
                            .get("id")
                            .and_then(|value| value.as_integer())
                            .map(|id| id.to_string())
                    });
                if let Some(key) = key {
                    entries.insert(key, entry.clone());
                }
            }
        }
        Ok(Self {
            entries,
            next_id: 0,
            ids: HashMap::new(),
        })
    }

    /// 从单个 TOML 文件加载通用注册表。
    ///
    /// 文件采用 `[[entry]]` 数组格式，每个条目以其 `name` 为键保留整张表。
    ///
    /// # 错误
    /// 文件缺失或解析失败返回 [`RegistryError::ParseError`]。
    pub fn from_toml_file(path: &Path) -> Result<Self, RegistryError> {
        let text = std::fs::read_to_string(path).map_err(|_| RegistryError::ParseError)?;
        Self::from_toml_str(&text)
    }

    /// 加载整个 `generic` 数据目录并合并所有 `.toml` 文件中的条目。
    ///
    /// 遍历目录下所有扩展名为 `toml` 的文件，逐个经 [`from_toml_file`](Self::from_toml_file)
    /// 解析后合并到同一张表；同名键以后读入的文件为准（后者覆盖前者）。
    ///
    /// # 错误
    /// 目录不可读或任一文件解析失败返回 [`RegistryError::ParseError`]。
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
        Ok(Self {
            entries: merged,
            next_id: 0,
            ids: HashMap::new(),
        })
    }

    /// 按 name 查询原始条目。
    pub fn get(&self, name: &str) -> Option<&toml::Value> {
        self.entries.get(name)
    }

    /// 条目数量。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否没有任何条目。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 覆盖或注册一个原始条目：name 已存在时替换值；不存在时新增条目。
    ///
    /// 本注册表以 name 为唯一键（无数值 id），故恒返回 `Ok(())`。
    pub fn override_value(&mut self, name: &str, value: toml::Value) -> Result<(), RegistryError> {
        self.entries.insert(name.to_string(), value);
        Ok(())
    }

    /// 注册或替换一个原始条目，返回分配到的自增 id。
    ///
    /// name 已存在（曾登记）时替换值并保留原 id、不推进自增序号；
    /// 不存在时插入新条目并分配新 id（`next_id + 1`）且推进。
    /// 此处的 id 仅作登记序号（通用条目自身不携带数值 id）。
    ///
    /// # 错误
    /// 仅当自增 id 溢出 `u32::MAX`（理论上不会发生）时返回
    /// [`RegistryError::IdOverflow`]。
    pub fn register_or_replace(
        &mut self,
        name: impl Into<String>,
        value: toml::Value,
    ) -> Result<u32, RegistryError> {
        let name = name.into();
        // 已登记：替换值并保留原 id，不推进自增序号。
        if let Some(&id) = self.ids.get(&name) {
            self.entries.insert(name, value);
            return Ok(id);
        }
        // 首次登记：分配新 id 并记录 name → id，供后续覆盖保留。
        let id = self
            .next_id
            .checked_add(1)
            .ok_or(RegistryError::IdOverflow)?;
        self.next_id = id;
        self.ids.insert(name.clone(), id);
        self.entries.insert(name, value);
        Ok(id)
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
                "generic_registry_test_{}_{}",
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

    /// 从字符串解析并提取某条目的 `type` 字段，便于断言兜底键内容完整。
    fn entry_type<'a>(registry: &'a GenericRegistry, key: &str) -> Option<&'a str> {
        registry
            .get(key)
            .and_then(|value| value.get("type"))
            .and_then(|value| value.as_str())
    }

    #[test]
    fn from_toml_str_uses_name_key_and_id_fallback_key() {
        // 同时包含：有 name 条目、无 name 仅有整数 id 条目、无 name 无 id 条目。
        let text = r#"
[[entry]]
name = "minecraft:ambient"
type = "ambient"

[[entry]]
id = 0
type = "master"

[[entry]]
type = "orphan"
"#;
        let registry = GenericRegistry::from_toml_str(text).unwrap();
        assert_eq!(registry.len(), 2);
        // name 优先作为键，整条表被保留。
        assert_eq!(entry_type(&registry, "minecraft:ambient"), Some("ambient"));
        // 无 name 时回退整数 id，0 转十进制字符串 "0"。
        assert_eq!(entry_type(&registry, "0"), Some("master"));
        // 无 name 无 id 的条目被跳过。
        assert!(registry.get("orphan").is_none());
    }

    #[test]
    fn from_toml_str_id_fallback_keeps_full_entry() {
        // 兜底键对应的表应为完整原始条目（含 type 字段），而非仅 id。
        let text = r#"
[[entry]]
id = 3
type = "weather"
"#;
        let registry = GenericRegistry::from_toml_str(text).unwrap();
        let entry = registry.get("3").unwrap();
        assert_eq!(entry.get("id").and_then(|v| v.as_integer()), Some(3));
        assert_eq!(entry.get("type").and_then(|v| v.as_str()), Some("weather"));
    }

    #[test]
    fn from_toml_str_empty_or_missing_entry_array_yields_empty() {
        // 空文本与无 entry 数组的文本都应得到空表而非错误。
        assert!(GenericRegistry::from_toml_str("").unwrap().is_empty());
        assert!(
            GenericRegistry::from_toml_str("name = \"x\"")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn load_directory_merges_all_toml_files_with_last_key_winning() {
        let dir = TempDir::new();
        dir.write(
            "first.toml",
            r#"
[[entry]]
name = "minecraft:shared"
type = "first"

[[entry]]
name = "minecraft:only_a"
type = "a"
"#,
        );
        dir.write(
            "second.toml",
            r#"
[[entry]]
id = 5
type = "five"

[[entry]]
name = "minecraft:shared"
type = "second"
"#,
        );
        // 非 toml 扩展名文件应被忽略。
        dir.write("ignore.txt", "name = \"minecraft:ignored\"");

        let registry = GenericRegistry::load_directory(dir.path()).unwrap();
        // shared / only_a / "5" 三条，ignore.txt 不参与。
        assert_eq!(registry.len(), 3);
        // 同名键：后读取的文件覆盖前者。
        assert_eq!(entry_type(&registry, "minecraft:shared"), Some("second"));
        assert_eq!(entry_type(&registry, "minecraft:only_a"), Some("a"));
        assert_eq!(entry_type(&registry, "5"), Some("five"));
    }

    #[test]
    fn load_directory_empty_dir_yields_empty_registry() {
        let dir = TempDir::new();
        let registry = GenericRegistry::load_directory(dir.path()).unwrap();
        assert!(registry.is_empty());
    }

    #[test]
    fn load_directory_missing_dir_returns_parse_error() {
        let missing = Path::new("does/not/exist/generic");
        assert!(matches!(
            GenericRegistry::load_directory(missing),
            Err(RegistryError::ParseError)
        ));
    }

    #[test]
    fn load_directory_bad_file_returns_parse_error() {
        let dir = TempDir::new();
        dir.write("broken.toml", "this is not = = valid toml @@@");
        assert!(matches!(
            GenericRegistry::load_directory(dir.path()),
            Err(RegistryError::ParseError)
        ));
    }

    #[test]
    fn override_value_replaces_existing_entry() {
        let mut registry = GenericRegistry::from_toml_str(
            r#"
[[entry]]
name = "minecraft:shared"
type = "old"
"#,
        )
        .unwrap();
        let replacement = toml::toml! {
            type = "new"
            strength = 3
        };
        registry
            .override_value("minecraft:shared", toml::Value::Table(replacement))
            .unwrap();
        assert_eq!(registry.len(), 1);
        assert_eq!(entry_type(&registry, "minecraft:shared"), Some("new"));
        assert_eq!(
            registry
                .get("minecraft:shared")
                .and_then(|v| v.get("strength"))
                .and_then(|v| v.as_integer()),
            Some(3)
        );
    }

    #[test]
    fn override_value_new_name_inserts() {
        let mut registry = GenericRegistry::new();
        registry
            .override_value("minecraft:brand_new", toml::toml! { type = "x" }.into())
            .unwrap();
        assert_eq!(registry.len(), 1);
        assert_eq!(entry_type(&registry, "minecraft:brand_new"), Some("x"));
    }

    #[test]
    fn register_or_replace_assigns_sequential_ids() {
        let mut registry = GenericRegistry::new();
        let first = registry
            .register_or_replace("minecraft:a", toml::toml! { type = "a" }.into())
            .unwrap();
        let second = registry
            .register_or_replace("minecraft:b", toml::toml! { type = "b" }.into())
            .unwrap();
        assert_eq!(first, 1);
        assert_eq!(second, 2);
        // 已存在：替换值并返回同一 id。
        let again = registry
            .register_or_replace("minecraft:a", toml::toml! { type = "a2" }.into())
            .unwrap();
        assert_eq!(again, 1);
        assert_eq!(entry_type(&registry, "minecraft:a"), Some("a2"));
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn register_or_replace_does_not_advance_next_id_on_overwrite() {
        let mut registry = GenericRegistry::new();
        // 前两次登记分配 id 1、2。
        let first = registry
            .register_or_replace("minecraft:a", toml::toml! { type = "a" }.into())
            .unwrap();
        let second = registry
            .register_or_replace("minecraft:b", toml::toml! { type = "b" }.into())
            .unwrap();
        assert_eq!(first, 1);
        assert_eq!(second, 2);
        // 覆盖已登记条目：返回原 id、条目数不变，且不推进自增序号。
        let overwritten = registry
            .register_or_replace("minecraft:a", toml::toml! { type = "a2" }.into())
            .unwrap();
        assert_eq!(overwritten, 1);
        assert_eq!(registry.len(), 2);
        // 后续新 name 应从推进后的 next_id 继续分配 → id = 3。
        let third = registry
            .register_or_replace("minecraft:c", toml::toml! { type = "c" }.into())
            .unwrap();
        assert_eq!(third, 3);
    }
}
