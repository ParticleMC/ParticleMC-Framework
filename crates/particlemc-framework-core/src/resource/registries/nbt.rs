//! 注册表 NBT 化：把注册数据转换为配置阶段的 RegistryData / UpdateTags 包。
//!
//! 对应 Minestom Java 的 `RegistryContainer` / `TagManager`：在配置阶段向客户端
//! 同步各注册表条目（RegistryData）与标签（UpdateTags）。本模块提供：
//!
//! - [`RegistrySnapshot`]：从各注册表 `Resource` 收集 name → 条目数据的映射；
//! - [`registry_data_packets`]：快照 → 每个同步注册表一个 [`RegistryData`] 包；
//! - [`update_tags_packet`]：标签注册表 → [`UpdateTags`] 包。
//!
//! 核心逻辑是 [`toml_value_to_nbt`]：把 TOML 条目数据无损映射为 NBT 标签，
//! 再经 `crate::protocol::nbt::encode_anonymous` 编码为包内字节。

use std::collections::HashMap;

use crate::protocol::nbt::{NbtTag, encode_anonymous};
use crate::protocol::packets::configuration::{
    RegistryData, RegistryEntry as RegistryDataEntry, TagEntry, TagRegistry as PacketTagRegistry,
    UpdateTags,
};

use super::generic::GenericRegistry;
use super::registry::{
    EntityTypeDefinition, GenericDefinition, Registry, RegistryEntry as RegistryEntryTrait,
};
use super::tags::TagRegistry as TagRegistryResource;

/// 单个同步注册表的收集结果。
struct SnapshotRegistry {
    /// 注册表 id（如 `minecraft:dimension_type`）。
    registry_id: String,
    /// 条目列表，保持 id 升序（协议按条目顺序隐式分配注册表 id）。
    entries: Vec<SnapshotEntry>,
}

/// 注册表条目：命名空间名称 + 原始 TOML 条目数据。
struct SnapshotEntry {
    /// 命名空间名称（如 `minecraft:plains`）。
    name: String,
    /// 整条 `[[entry]]` 的原始 TOML 表。
    value: toml::Value,
}

/// 注册数据快照：收集各注册表 Resource 的 name → 条目数据映射。
///
/// 由装配层（插件）在持有全部注册表 Resource 后构建：具名注册表经
/// [`push_named_definitions`](Self::push_named_definitions) /
/// [`push_entity_types`](Self::push_entity_types) 收集，通用注册表经
/// [`push_generic`](Self::push_generic) 收集。之后交给 [`registry_data_packets`]
/// 生成包。
#[derive(Default)]
pub struct RegistrySnapshot {
    /// 已收集的同步注册表（保持收集顺序）。
    registries: Vec<SnapshotRegistry>,
}

impl RegistrySnapshot {
    /// 创建空的注册数据快照。
    pub fn new() -> Self {
        Self::default()
    }

    /// 从通用注册表收集条目，挂到指定的注册表 id 下。
    ///
    /// 注意：`GenericRegistry::load_directory` 合并 generic/ 下全部文件时会丢失
    /// 文件边界，无法分辨某条目属于哪个同步注册表。因此本方法要求传入**单一
    /// 注册表**的实例（只包含一个 generic/*.toml 文件内容的 `GenericRegistry`），
    /// 由装配层按文件逐个加载后调用。
    pub fn push_generic(&mut self, registry_id: &str, registry: &GenericRegistry) {
        let entries = registry
            .entries
            .iter()
            .map(|(name, value)| SnapshotEntry {
                name: name.clone(),
                value: value.clone(),
            })
            .collect();
        self.registries.push(SnapshotRegistry {
            registry_id: registry_id.to_string(),
            entries,
        });
    }

    /// 从 `Registry<GenericDefinition>` 承载的具名注册表收集条目。
    ///
    /// 适用于维度类型、生物群系、伤害类型、附魔、流体、粒子、音效事件、
    /// 药水效果等世界类注册表（挂到调用方指定的注册表 id 下）。
    pub fn push_named_definitions(
        &mut self,
        registry_id: &str,
        registry: &Registry<GenericDefinition>,
    ) {
        self.push_named_entries(registry_id, registry);
    }

    /// 从实体类型注册表收集条目（挂到 `minecraft:entity_type`）。
    pub fn push_entity_types(&mut self, registry: &Registry<EntityTypeDefinition>) {
        self.push_named_entries("minecraft:entity_type", registry);
    }

    /// 已收集的同步注册表数量（含空注册表）。
    pub fn registry_count(&self) -> usize {
        self.registries.len()
    }

    /// 从 Minestom 数据目录（`resources/data`）构建完整注册表快照。
    ///
    /// 与测试辅助 [`tests::build_snapshot`] 使用相同的注册表映射表，供插件装配
    /// 复用：具名注册表（维度/生物群系/伤害/附魔/流体/粒子/音效/药水）+ 实体类型
    /// + 19 个 generic 文件注册表。文件缺失或解析失败时回退为空注册表（不 panic）。
    pub fn from_data_dir(data_dir: &std::path::Path) -> Self {
        let mut snapshot = RegistrySnapshot::new();

        let named: &[(&str, &str)] = &[
            ("minecraft:dimension_type", "dimension_types.toml"),
            ("minecraft:worldgen/biome", "biomes.toml"),
            ("minecraft:damage_type", "damage_types.toml"),
            ("minecraft:enchantment", "enchantments.toml"),
            ("minecraft:fluid", "fluids.toml"),
            ("minecraft:particle_type", "particles.toml"),
            ("minecraft:sound_event", "sound_events.toml"),
            ("minecraft:potion_effect", "potion_effects.toml"),
        ];
        for (registry_id, file) in named {
            let registry = Registry::<GenericDefinition>::from_toml_file(&data_dir.join(file))
                .unwrap_or_default();
            snapshot.push_named_definitions(registry_id, &registry);
        }
        snapshot.push_entity_types(
            &Registry::<EntityTypeDefinition>::from_toml_file(&data_dir.join("entity_types.toml"))
                .unwrap_or_default(),
        );

        let generic = data_dir.join("generic");
        let generic_files: &[(&str, &str)] = &[
            ("minecraft:attribute", "attribute.toml"),
            ("minecraft:banner_pattern", "banner_pattern.toml"),
            ("minecraft:chat_type", "chat_type.toml"),
            ("minecraft:instrument", "instrument.toml"),
            ("minecraft:jukebox_song", "jukebox_song.toml"),
            ("minecraft:painting_variant", "painting_variant.toml"),
            ("minecraft:trim_material", "trim_material.toml"),
            ("minecraft:trim_pattern", "trim_pattern.toml"),
            ("minecraft:cat_variant", "cat_variant.toml"),
            ("minecraft:chicken_variant", "chicken_variant.toml"),
            ("minecraft:cow_variant", "cow_variant.toml"),
            ("minecraft:frog_variant", "frog_variant.toml"),
            ("minecraft:pig_variant", "pig_variant.toml"),
            ("minecraft:wolf_variant", "wolf_variant.toml"),
            ("minecraft:wolf_sound_variant", "wolf_sound_variant.toml"),
            ("minecraft:dialog", "dialog.toml"),
            ("minecraft:game_event", "game_event.toml"),
            ("minecraft:world_clock", "world_clock.toml"),
            ("minecraft:timeline", "timeline.toml"),
        ];
        for (registry_id, file) in generic_files {
            let registry = GenericRegistry::from_toml_file(&generic.join(file)).unwrap_or_default();
            snapshot.push_generic(registry_id, &registry);
        }
        snapshot
    }

    /// 内部：按 id 升序遍历具名注册表，把每条目转为原始 TOML 表收集。
    fn push_named_entries<T>(&mut self, registry_id: &str, registry: &Registry<T>)
    where
        T: RegistryEntryTrait + ToTomlValue,
    {
        let mut entries = Vec::with_capacity(registry.len());
        for id in 0..registry.len() {
            let Ok(id) = u32::try_from(id) else { break };
            let Some(name) = registry.get_name(id) else {
                continue;
            };
            let Some(value) = registry.get(id) else {
                continue;
            };
            entries.push(SnapshotEntry {
                name: name.to_string(),
                value: value.to_toml_value(),
            });
        }
        self.registries.push(SnapshotRegistry {
            registry_id: registry_id.to_string(),
            entries,
        });
    }
}

/// 具名注册表条目 → 原始 TOML 表，供快照以 `toml::Value` 统一存盘。
trait ToTomlValue {
    /// 把条目结构还原为完整的 `[[entry]]` TOML 表。
    fn to_toml_value(&self) -> toml::Value;
}

impl ToTomlValue for GenericDefinition {
    fn to_toml_value(&self) -> toml::Value {
        // 反序列化时 flatten 收集的是 HashMap，而 toml::Value::Table 要求
        // toml::map::Map，经 FromIterator 转换回映射表
        let mut table: toml::map::Map<String, toml::Value> =
            self.extra.clone().into_iter().collect();
        table.insert("name".to_string(), toml::Value::String(self.name.clone()));
        if let Some(id) = self.id {
            table.insert("id".to_string(), toml::Value::Integer(i64::from(id)));
        }
        toml::Value::Table(table)
    }
}

impl ToTomlValue for EntityTypeDefinition {
    fn to_toml_value(&self) -> toml::Value {
        let mut table: toml::map::Map<String, toml::Value> =
            self.extra.clone().into_iter().collect();
        table.insert("name".to_string(), toml::Value::String(self.name.clone()));
        if let Some(id) = self.id {
            table.insert("id".to_string(), toml::Value::Integer(i64::from(id)));
        }
        if let Some(key) = &self.translation_key {
            table.insert(
                "translationKey".to_string(),
                toml::Value::String(key.clone()),
            );
        }
        if let Some(width) = self.width {
            table.insert("width".to_string(), toml::Value::Float(width));
        }
        if let Some(height) = self.height {
            table.insert("height".to_string(), toml::Value::Float(height));
        }
        toml::Value::Table(table)
    }
}

/// 把注册数据快照转换为配置阶段要发送的 RegistryData 包列表。
///
/// 每个同步注册表生成一个包；条目值经 TOML → NBT 转换后用 anonymous NBT 编码
/// 为原始字节填入 `RegistryDataEntry.value`。空注册表（无任何条目）被跳过，
/// 不生成包、不 panic。
pub fn registry_data_packets(snapshot: &RegistrySnapshot) -> Vec<RegistryData> {
    snapshot
        .registries
        .iter()
        .filter_map(|registry| {
            if registry.entries.is_empty() {
                return None;
            }
            let entries = registry
                .entries
                .iter()
                .filter_map(|entry| {
                    let nbt = entry_compound(&entry.value);
                    encode_anonymous(&nbt).ok().map(|bytes| RegistryDataEntry {
                        key: entry.name.clone(),
                        value: Some(bytes),
                    })
                })
                .collect();
            Some(RegistryData {
                registry_id: registry.registry_id.clone(),
                entries,
            })
        })
        .collect()
}

/// 把整条 `[[entry]]` 的 TOML 表转换为条目值 NBT（Compound）。
///
/// 剔除 `name` 与 `id` 两个字段：`name` 是条目键（已在 `RegistryDataEntry.key`
/// 中携带），`id` 是注册表内的位置序号，vanilla 注册表同步均不发送这两者。
/// 非表输入（数据异常）回退为空 Compound，保证编码不失败。
fn entry_compound(value: &toml::Value) -> NbtTag {
    let NbtTag::Compound(entries) = toml_value_to_nbt(value) else {
        return NbtTag::Compound(Vec::new());
    };
    NbtTag::Compound(
        entries
            .into_iter()
            .filter(|(key, _)| key != "name" && key != "id")
            .collect(),
    )
}

/// 把 TOML 值转换为 NBT 标签。
///
/// 类型映射的设计意图：
///
/// - **Boolean → Byte(0/1)**：NBT 没有布尔类型，vanilla 注册表同步以字节 0/1
///   表达布尔字段（如 `has_ceiling`、`has_precipitation`），与线格式一致；
/// - **Integer → Int/Long**：TOML 整数是 i64，能收进 i32 的用 Int（贴近 vanilla
///   字段宽度），超出则用 Long 避免溢出丢失；
/// - **Float → Double**：注册数据多为 f64（环境光、坐标缩放等），Double 无损承载；
/// - **Array → List**：NBT List 要求元素类型统一，混合数组的兜底见
///   [`toml_array_to_list`]；
/// - **Table → Compound**：递归展开键值对；
/// - **Datetime → String**：TOML 特有类型，数据中几乎不出现，转字符串兜底。
fn toml_value_to_nbt(value: &toml::Value) -> NbtTag {
    match value {
        toml::Value::String(text) => NbtTag::String(text.clone()),
        toml::Value::Integer(num) => match i32::try_from(*num) {
            Ok(short) => NbtTag::Int(short),
            Err(_) => NbtTag::Long(*num),
        },
        toml::Value::Float(num) => NbtTag::Double(*num),
        toml::Value::Boolean(flag) => NbtTag::Byte(if *flag { 1 } else { 0 }),
        toml::Value::Array(items) => toml_array_to_list(items),
        toml::Value::Table(table) => NbtTag::Compound(
            table
                .iter()
                .map(|(key, value)| (key.clone(), toml_value_to_nbt(value)))
                .collect(),
        ),
        toml::Value::Datetime(datetime) => NbtTag::String(datetime.to_string()),
    }
}

/// 把 TOML 数组转换为 NBT List。
///
/// NBT 协议要求 List 内所有元素类型一致；TOML 数组则可能为空或类型混杂。
/// 空数组返回空 List（元素类型声明为 TAG_End）；元素类型一致时原样保留；
/// 类型混杂时把各元素转为字符串（[`tag_to_string`]），以可读形式保住数据，
/// 避免编码出的 List 元素类型不一致导致客户端解码失败。
fn toml_array_to_list(items: &[toml::Value]) -> NbtTag {
    let tags: Vec<NbtTag> = items.iter().map(toml_value_to_nbt).collect();
    if tags.is_empty() {
        return NbtTag::List(Vec::new());
    }
    let first_kind = tags.first().map(std::mem::discriminant);
    let uniform = tags
        .iter()
        .map(std::mem::discriminant)
        .all(|kind| Some(kind) == first_kind);
    if uniform {
        NbtTag::List(tags)
    } else {
        NbtTag::List(
            tags.iter()
                .map(|tag| NbtTag::String(tag_to_string(tag)))
                .collect(),
        )
    }
}

/// 把 NBT 标签转为字符串形式，供混合类型数组统一兜底。
fn tag_to_string(tag: &NbtTag) -> String {
    match tag {
        NbtTag::Byte(value) => value.to_string(),
        NbtTag::Short(value) => value.to_string(),
        NbtTag::Int(value) => value.to_string(),
        NbtTag::Long(value) => value.to_string(),
        NbtTag::Float(value) => value.to_string(),
        NbtTag::Double(value) => value.to_string(),
        NbtTag::String(value) => value.clone(),
        NbtTag::ByteArray(values) => format!("byte_array({})", values.len()),
        NbtTag::List(values) => format!("list({})", values.len()),
        NbtTag::Compound(values) => format!("compound({})", values.len()),
        NbtTag::IntArray(values) => format!("int_array({})", values.len()),
        NbtTag::LongArray(values) => format!("long_array({})", values.len()),
    }
}

/// UpdateTags 单组的占位注册表 id。
///
/// 见 [`update_tags_packet`] 的设计说明：平铺的 `TagRegistry` 无法还原每个标签
/// 所属的注册表，此处以 `minecraft:block` 占位，由装配层按文件加载标签后替换。
const TAGS_PLACEHOLDER_REGISTRY: &str = "minecraft:block";

/// 把标签注册表转换为配置阶段要发送的 UpdateTags 包。
///
/// `TagRegistry` 是「标签名 → 成员名列表」的平铺映射：`load_directory` 合并
/// tags/ 下各文件时丢失了文件边界（blocks 与 items 存在同名标签），成员也以
/// 名称而非协议要求的数值 id 保存。注册表分组与成员 id 解析都依赖具体注册表
/// 数据，超出本函数签名可见范围。
///
/// 因此本函数完成**结构转换**：每个标签映射为一个 [`TagEntry`]（`identifier`
/// 保留标签名），成员名经稳定的排序序数暂代数值 id（[`build_member_id_map`]，
/// 同名恒同值；`#` 前缀的标签引用不参与计数，客户端会从同注册表其它标签解析）；
/// 全部标签置于单个注册表组。真实注册表 id 与分组由装配层在持有
/// `RegistrySnapshot` 后替换。
pub fn update_tags_packet(tags: &TagRegistryResource) -> UpdateTags {
    let member_ids = build_member_id_map(tags);
    let mut entries: Vec<TagEntry> = tags
        .tags
        .iter()
        .map(|(name, values)| TagEntry {
            identifier: name.clone(),
            entries: values
                .iter()
                .filter(|value| !value.starts_with('#'))
                .filter_map(|value| member_ids.get(value.as_str()).copied())
                .collect(),
        })
        .collect();
    // HashMap 迭代无序，按 identifier 排序保证输出确定性
    entries.sort_by(|a, b| a.identifier.cmp(&b.identifier));
    UpdateTags {
        registries: vec![PacketTagRegistry {
            registry: TAGS_PLACEHOLDER_REGISTRY.to_string(),
            tags: entries,
        }],
    }
}

/// 构建「成员名 → 稳定序数 id」的映射。
///
/// 收集全部非 `#` 引用成员名，排序去重后按下标分配 id，保证同名恒同值、
/// 输出确定性。序数仅作结构占位，非真实注册表 id。
fn build_member_id_map(tags: &TagRegistryResource) -> HashMap<String, i32> {
    let mut names: Vec<&str> = tags
        .tags
        .values()
        .flat_map(|values| values.iter().map(String::as_str))
        .filter(|value| !value.starts_with('#'))
        .collect();
    names.sort_unstable();
    names.dedup();
    names
        .into_iter()
        .enumerate()
        .filter_map(|(index, name)| i32::try_from(index).ok().map(|id| (name.to_string(), id)))
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::protocol::nbt::decode_root;

    /// 返回 Minestom 数据目录（相对当前 crate 的 `CARGO_MANIFEST_DIR`）。
    fn data_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/data")
    }

    /// 加载全部同步注册表构建快照：8 个 GenericDefinition 具名 + entity_type + 19 个通用。
    fn build_snapshot() -> RegistrySnapshot {
        let mut snapshot = RegistrySnapshot::new();
        let data = data_dir();

        // 具名注册表（Registry<GenericDefinition> 承载）
        let named: &[(&str, &str)] = &[
            ("minecraft:dimension_type", "dimension_types.toml"),
            ("minecraft:worldgen/biome", "biomes.toml"),
            ("minecraft:damage_type", "damage_types.toml"),
            ("minecraft:enchantment", "enchantments.toml"),
            ("minecraft:fluid", "fluids.toml"),
            ("minecraft:particle_type", "particles.toml"),
            ("minecraft:sound_event", "sound_events.toml"),
            ("minecraft:potion_effect", "potion_effects.toml"),
        ];
        for (registry_id, file) in named {
            let registry = Registry::<GenericDefinition>::from_toml_file(&data.join(file)).unwrap();
            snapshot.push_named_definitions(registry_id, &registry);
        }
        snapshot.push_entity_types(
            &Registry::<EntityTypeDefinition>::from_toml_file(&data.join("entity_types.toml"))
                .unwrap(),
        );

        // 通用注册表（每个 generic/*.toml 对应一个同步注册表）
        let generic = data.join("generic");
        let generic_files: &[(&str, &str)] = &[
            ("minecraft:attribute", "attribute.toml"),
            ("minecraft:banner_pattern", "banner_pattern.toml"),
            ("minecraft:chat_type", "chat_type.toml"),
            ("minecraft:instrument", "instrument.toml"),
            ("minecraft:jukebox_song", "jukebox_song.toml"),
            ("minecraft:painting_variant", "painting_variant.toml"),
            ("minecraft:trim_material", "trim_material.toml"),
            ("minecraft:trim_pattern", "trim_pattern.toml"),
            ("minecraft:cat_variant", "cat_variant.toml"),
            ("minecraft:chicken_variant", "chicken_variant.toml"),
            ("minecraft:cow_variant", "cow_variant.toml"),
            ("minecraft:frog_variant", "frog_variant.toml"),
            ("minecraft:pig_variant", "pig_variant.toml"),
            ("minecraft:wolf_variant", "wolf_variant.toml"),
            ("minecraft:wolf_sound_variant", "wolf_sound_variant.toml"),
            ("minecraft:dialog", "dialog.toml"),
            ("minecraft:game_event", "game_event.toml"),
            ("minecraft:world_clock", "world_clock.toml"),
            ("minecraft:timeline", "timeline.toml"),
        ];
        for (registry_id, file) in generic_files {
            let registry = GenericRegistry::from_toml_file(&generic.join(file)).unwrap();
            snapshot.push_generic(registry_id, &registry);
        }
        snapshot
    }

    /// 从生成的包列表中按注册表 id 查找。
    fn find_packet<'a>(packets: &'a [RegistryData], registry_id: &str) -> &'a RegistryData {
        packets
            .iter()
            .find(|packet| packet.registry_id == registry_id)
            .expect("应存在该注册表包")
    }

    /// 按条目键查找某条目的 NBT 值。
    fn entry_value<'a>(packet: &'a RegistryData, key: &str) -> &'a [u8] {
        packet
            .entries
            .iter()
            .find(|entry| entry.key == key)
            .map(|entry| entry.value.as_deref().unwrap())
            .expect("应存在该条目")
    }

    #[test]
    fn registry_data_packets_covers_sync_registries() {
        let snapshot = build_snapshot();
        let packets = registry_data_packets(&snapshot);

        // 同步注册表数应 ≥ 15
        assert!(
            packets.len() >= 15,
            "同步注册表应 ≥ 15，实际为 {}",
            packets.len()
        );

        // dimension_type 至少 1 条
        let dimension = find_packet(&packets, "minecraft:dimension_type");
        assert!(!dimension.entries.is_empty(), "dimension_type 应至少 1 条");

        // biome 注册表含 minecraft:plains
        let biome = find_packet(&packets, "minecraft:worldgen/biome");
        assert!(
            biome
                .entries
                .iter()
                .any(|entry| entry.key == "minecraft:plains"),
            "biome 应含 minecraft:plains"
        );

        // damage_type 含 25 个标准类型
        let damage = find_packet(&packets, "minecraft:damage_type");
        let standard = [
            "cactus",
            "campfire",
            "cramming",
            "dragon_breath",
            "drown",
            "dry_out",
            "ender_pearl",
            "fall",
            "fly_into_wall",
            "freeze",
            "generic",
            "generic_kill",
            "hot_floor",
            "in_fire",
            "in_wall",
            "lava",
            "lightning_bolt",
            "magic",
            "on_fire",
            "out_of_world",
            "outside_border",
            "stalagmite",
            "starve",
            "sweet_berry_bush",
            "wither",
        ];
        for name in standard {
            let key = format!("minecraft:{name}");
            assert!(
                damage.entries.iter().any(|entry| entry.key == key),
                "damage_type 应含标准类型 {key}"
            );
        }
    }

    #[test]
    fn registry_data_packet_values_roundtrip_as_nbt() {
        let snapshot = build_snapshot();
        for packet in registry_data_packets(&snapshot) {
            for entry in &packet.entries {
                let bytes = entry.value.as_deref().expect("条目 value 应已 NBT 化");
                // anonymous NBT 无根名头，补 `0x0a + 0x00` 后应可完整解码为 Compound
                let mut full = vec![0x0a, 0x00];
                full.extend_from_slice(bytes);
                let (_, tag) = decode_root(&full).unwrap();
                assert!(
                    matches!(tag, NbtTag::Compound(_)),
                    "条目 {} 的值应为 Compound",
                    entry.key
                );
            }
        }
    }

    #[test]
    fn registry_data_packets_skips_empty_registries() {
        let snapshot = RegistrySnapshot::new();
        assert!(registry_data_packets(&snapshot).is_empty());
    }

    #[test]
    fn dimension_type_value_contains_vanilla_fields() {
        let snapshot = build_snapshot();
        let packets = registry_data_packets(&snapshot);
        let dimension = find_packet(&packets, "minecraft:dimension_type");
        let bytes = entry_value(dimension, "minecraft:overworld");
        let mut full = vec![0x0a, 0x00];
        full.extend_from_slice(bytes);
        let (_, NbtTag::Compound(entries)) = decode_root(&full).unwrap() else {
            panic!("dimension_type 值应为 Compound");
        };
        // name/id 不应出现在值 NBT 中（name 是键，id 是位置序号）
        for (key, _) in &entries {
            assert!(key != "name" && key != "id", "值 NBT 不应含字段 {key}");
        }
        // vanilla 维度类型必备字段存在
        let keys: Vec<&str> = entries.iter().map(|(key, _)| key.as_str()).collect();
        for required in ["min_y", "height", "coordinate_scale", "ambient_light"] {
            assert!(keys.contains(&required), "缺少字段 {required}");
        }
    }

    #[test]
    fn update_tags_packet_structure_is_correct() {
        let mut tags = TagRegistryResource::default();
        tags.tags.insert(
            "minecraft:acacia_logs".to_string(),
            vec![
                "minecraft:acacia_log".to_string(),
                "minecraft:acacia_wood".to_string(),
            ],
        );
        tags.tags.insert(
            "minecraft:planks".to_string(),
            vec!["minecraft:oak_planks".to_string()],
        );
        tags.tags
            .insert("minecraft:empty_tag".to_string(), Vec::new());
        tags.tags.insert(
            "minecraft:with_reference".to_string(),
            vec!["#minecraft:logs".to_string()],
        );

        let packet = update_tags_packet(&tags);

        // 单一注册表组，且注册表 id 非空
        assert_eq!(packet.registries.len(), 1);
        let group = packet.registries.first().unwrap();
        assert!(!group.registry.is_empty());

        // 标签数 = 输入标签数
        assert_eq!(group.tags.len(), 4);

        // 每个标签条目结构完整：identifier 保留标签名、entries 为 i32 列表
        for tag in &group.tags {
            assert!(tag.identifier.starts_with("minecraft:"));
        }

        // 直接成员解析为稳定序数 id：# 引用被排除、空标签 entries 为空
        let acacia = group
            .tags
            .iter()
            .find(|tag| tag.identifier == "minecraft:acacia_logs")
            .unwrap();
        assert_eq!(acacia.entries.len(), 2);
        assert_ne!(acacia.entries.first(), acacia.entries.get(1));
        let with_reference = group
            .tags
            .iter()
            .find(|tag| tag.identifier == "minecraft:with_reference")
            .unwrap();
        assert!(with_reference.entries.is_empty(), "# 引用不应计入 entries");
        let empty = group
            .tags
            .iter()
            .find(|tag| tag.identifier == "minecraft:empty_tag")
            .unwrap();
        assert!(empty.entries.is_empty());
    }

    #[test]
    fn update_tags_packet_is_deterministic() {
        let mut tags = TagRegistryResource::default();
        tags.tags
            .insert("minecraft:zzz".to_string(), vec!["minecraft:z".to_string()]);
        tags.tags
            .insert("minecraft:aaa".to_string(), vec!["minecraft:a".to_string()]);
        let first = update_tags_packet(&tags);
        let second = update_tags_packet(&tags);
        // 输出应按 identifier 排序、可复现
        assert_eq!(first, second);
        let group = first.registries.first().unwrap();
        assert_eq!(group.tags.first().unwrap().identifier, "minecraft:aaa");
        assert_eq!(group.tags.get(1).unwrap().identifier, "minecraft:zzz");
        // 同名成员在不同标签中映射到同一 id
        let ids: Vec<i32> = group
            .tags
            .iter()
            .flat_map(|tag| tag.entries.iter().copied())
            .collect();
        let distinct = {
            let mut sorted = ids.clone();
            sorted.sort_unstable();
            sorted.dedup();
            sorted
        };
        assert_eq!(ids.len(), 2);
        assert_eq!(distinct.len(), 2, "不同成员名应映射到不同 id");
    }

    #[test]
    fn toml_value_to_nbt_maps_all_primitive_types() {
        let table: toml::Value = toml::from_str(
            r#"
string = "hello"
int = 42
big_int = 3000000000
float = 1.5
boolean = true
array = [1, 2, 3]
nested = { key = "value" }
"#,
        )
        .unwrap();
        let NbtTag::Compound(entries) = toml_value_to_nbt(&table) else {
            panic!("表应转为 Compound");
        };
        let get = |key: &str| entries.iter().find(|(k, _)| k == key).map(|(_, v)| v);

        assert_eq!(get("string"), Some(&NbtTag::String("hello".into())));
        assert_eq!(get("int"), Some(&NbtTag::Int(42)));
        // 超出 i32 的整数用 Long 防丢失
        assert_eq!(get("big_int"), Some(&NbtTag::Long(3_000_000_000)));
        assert_eq!(get("float"), Some(&NbtTag::Double(1.5)));
        // 布尔以 Byte(0/1) 表达
        assert_eq!(get("boolean"), Some(&NbtTag::Byte(1)));
        assert_eq!(
            get("array"),
            Some(&NbtTag::List(vec![
                NbtTag::Int(1),
                NbtTag::Int(2),
                NbtTag::Int(3)
            ]))
        );
        assert_eq!(
            get("nested"),
            Some(&NbtTag::Compound(vec![(
                "key".into(),
                NbtTag::String("value".into())
            )]))
        );
    }

    #[test]
    fn toml_value_to_nbt_unifies_mixed_arrays_to_strings() {
        let table: toml::Value = toml::from_str("mixed = [1, \"two\", true]").unwrap();
        let NbtTag::Compound(entries) = toml_value_to_nbt(&table) else {
            panic!("表应转为 Compound");
        };
        let mixed = entries
            .iter()
            .find(|(key, _)| key == "mixed")
            .map(|(_, value)| value)
            .unwrap();
        // 混合类型数组统一为字符串 List，避免 NBT List 元素类型不一致
        assert_eq!(
            mixed,
            &NbtTag::List(vec![
                NbtTag::String("1".into()),
                NbtTag::String("two".into()),
                NbtTag::String("1".into()),
            ])
        );
    }
}
