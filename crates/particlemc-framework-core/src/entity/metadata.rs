// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 实体 / 生物元数据类型（变更标识符：`complete-missing-subsystems`）。
//!
//! 对应 Java `net.minestom.server.entity.metadata` 包（1.21.11，~190 文件）。
//! 本模块只对齐字段 / 类别 / 默认值，不复制翻译 Java 实现。
//!
//! # 结构
//!
//! - [`EntityMetaType`]：类别枚举。覆盖抽象层（Entity / LivingEntity / Mob /
//!   PathfinderMob / AgeableMob / AbstractVehicle 等）与全部具体生物类别，
//!   由数据表宏生成 `from_name` / `as_str` / `all`。
//! - [`TypedMeta`]：类型化 metadata 与 [`EntityMetadataMap`] 的互转 trait。
//! - 手写核心 struct（约 14 类，含玩家 / 常见生物）实现 [`TypedMeta`]：
//!   [`BaseEntityMeta`] / [`LivingEntityMeta`] / [`MobMeta`] / [`AnimalMeta`] /
//!   [`ZombieMeta`] / [`SkeletonMeta`] / [`CreeperMeta`] / [`PigMeta`] /
//!   [`WolfMeta`] / [`VillagerMeta`] / [`IronGolemMeta`] / [`SlimeMeta`] /
//!   [`BatMeta`] / [`FishingHookMeta`]。其余类型（数量庞大、字段多为注册表 /
//!   复合类型）由既有 [`EntityMetadataMap`] 兜底承载。
//! - [`EntityMetadataMap`] 扩展：`to_entries` / `from_entries` 作为与 play 层
//!   `entity_metadata`（0x61/0x62）包的条目互转桥梁。
//!
//! # index 约定
//!
//! 与 Java `MetadataDef` 一致，各类型使用「类型内局部 index」（0 起）。协议
//! 线格式的全局 index 展开由后续 0x62 包编码接入职责负责（本任务不接入包
//! 编码，仅提供值类型 + map 互转）。

use crate::component::{EntityMetadataMap, EntityMetadataValue};

// ---------------------------------------------------------------------------
// 类别枚举：全量注册表（对齐 Java 类名，宏生成 from_name / as_str / all）
// ---------------------------------------------------------------------------

/// 生成 [`EntityMetaType`] 枚举及其注册表辅助函数。
///
/// 每个条目 `变体 = "Java 基名"`（如 `Zombie = "Zombie"` 对应 Java
/// `ZombieMeta`）。`from_name` 同时接受 Java 类名（`ZombieMeta`）与基名
/// （`Zombie`）。
macro_rules! meta_type_table {
    ($($variant:ident = $base:literal),* $(,)?) => {
        /// 实体元数据类别（对齐 Java `net.minestom.server.entity.metadata`）。
        ///
        /// 变体命名去掉 Java 类名后缀 `Meta`（如 `Zombie` 对应 `ZombieMeta`）。
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum EntityMetaType {
            $($variant),*
        }

        impl EntityMetaType {
            /// 按类名 / 基名解析类别；未知名返回 `None`。
            ///
            /// 接受两种写法：Java 类名 `"ZombieMeta"` 或基名 `"Zombie"`。
            pub fn from_name(name: &str) -> Option<Self> {
                let base = name.strip_suffix("Meta").unwrap_or(name);
                match base {
                    $($base => Some(Self::$variant),)*
                    _ => None,
                }
            }

            /// 返回对应 Java 类名（如 `"ZombieMeta"`）。
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => concat!($base, "Meta"),)*
                }
            }

            /// 全部类别（注册表全量）。
            pub fn all() -> &'static [Self] {
                const ALL: &[EntityMetaType] = &[$(EntityMetaType::$variant),*];
                ALL
            }
        }
    };
}

meta_type_table! {
    // 根 / 抽象层
    Entity = "Entity",
    LivingEntity = "LivingEntity",
    Mob = "Mob",
    PathfinderMob = "PathfinderMob",
    AgeableMob = "AgeableMob",
    AbstractVehicle = "AbstractVehicle",
    // ambient
    AmbientCreature = "AmbientCreature",
    Bat = "Bat",
    // animal
    AbstractHorse = "AbstractHorse",
    AbstractNautilus = "AbstractNautilus",
    Animal = "Animal",
    Armadillo = "Armadillo",
    Bee = "Bee",
    CamelHusk = "CamelHusk",
    Camel = "Camel",
    ChestedHorse = "ChestedHorse",
    Chicken = "Chicken",
    Cow = "Cow",
    Donkey = "Donkey",
    Fox = "Fox",
    Frog = "Frog",
    Goat = "Goat",
    HappyGhast = "HappyGhast",
    Hoglin = "Hoglin",
    Horse = "Horse",
    Llama = "Llama",
    Mooshroom = "Mooshroom",
    Mule = "Mule",
    Nautilus = "Nautilus",
    Ocelot = "Ocelot",
    Panda = "Panda",
    Pig = "Pig",
    PolarBear = "PolarBear",
    Rabbit = "Rabbit",
    Sheep = "Sheep",
    SkeletonHorse = "SkeletonHorse",
    Sniffer = "Sniffer",
    Strider = "Strider",
    Turtle = "Turtle",
    ZombieHorse = "ZombieHorse",
    ZombieNautilus = "ZombieNautilus",
    // animal.tameable
    Cat = "Cat",
    Parrot = "Parrot",
    TameableAnimal = "TameableAnimal",
    Wolf = "Wolf",
    // avatar
    Avatar = "Avatar",
    Mannequin = "Mannequin",
    Player = "Player",
    // display
    AbstractDisplay = "AbstractDisplay",
    BlockDisplay = "BlockDisplay",
    ItemDisplay = "ItemDisplay",
    TextDisplay = "TextDisplay",
    // flying
    Flying = "Flying",
    Ghast = "Ghast",
    Phantom = "Phantom",
    // golem
    AbstractGolem = "AbstractGolem",
    CopperGolem = "CopperGolem",
    IronGolem = "IronGolem",
    Shulker = "Shulker",
    SnowGolem = "SnowGolem",
    // item
    EyeOfEnder = "EyeOfEnder",
    Fireball = "Fireball",
    ItemEntity = "ItemEntity",
    LingeringPotion = "LingeringPotion",
    SmallFireball = "SmallFireball",
    Snowball = "Snowball",
    SplashPotion = "SplashPotion",
    ThrownEgg = "ThrownEgg",
    ThrownEnderPearl = "ThrownEnderPearl",
    ThrownExperienceBottle = "ThrownExperienceBottle",
    ThrownItemProjectile = "ThrownItemProjectile",
    // minecart
    AbstractMinecartContainer = "AbstractMinecartContainer",
    AbstractMinecart = "AbstractMinecart",
    ChestMinecart = "ChestMinecart",
    CommandBlockMinecart = "CommandBlockMinecart",
    FurnaceMinecart = "FurnaceMinecart",
    HopperMinecart = "HopperMinecart",
    Minecart = "Minecart",
    SpawnerMinecart = "SpawnerMinecart",
    TntMinecart = "TntMinecart",
    // monster
    BasePiglin = "BasePiglin",
    Blaze = "Blaze",
    Breeze = "Breeze",
    CaveSpider = "CaveSpider",
    Creaking = "Creaking",
    Creeper = "Creeper",
    ElderGuardian = "ElderGuardian",
    Enderman = "Enderman",
    Endermite = "Endermite",
    Giant = "Giant",
    Guardian = "Guardian",
    Monster = "Monster",
    PiglinBrute = "PiglinBrute",
    Piglin = "Piglin",
    Silverfish = "Silverfish",
    Spider = "Spider",
    Vex = "Vex",
    Warden = "Warden",
    Wither = "Wither",
    Zoglin = "Zoglin",
    // monster.raider
    AbstractIllager = "AbstractIllager",
    Evoker = "Evoker",
    Illusioner = "Illusioner",
    Pillager = "Pillager",
    Raider = "Raider",
    Ravager = "Ravager",
    SpellcasterIllager = "SpellcasterIllager",
    Vindicator = "Vindicator",
    Witch = "Witch",
    // monster.skeleton
    AbstractSkeleton = "AbstractSkeleton",
    Bogged = "Bogged",
    Parched = "Parched",
    Skeleton = "Skeleton",
    Stray = "Stray",
    WitherSkeleton = "WitherSkeleton",
    // monster.zombie
    Drowned = "Drowned",
    Husk = "Husk",
    Zombie = "Zombie",
    ZombieVillager = "ZombieVillager",
    ZombifiedPiglin = "ZombifiedPiglin",
    // other
    Allay = "Allay",
    AreaEffectCloud = "AreaEffectCloud",
    ArmorStand = "ArmorStand",
    Boat = "Boat",
    EndCrystal = "EndCrystal",
    EnderDragon = "EnderDragon",
    EvokerFangs = "EvokerFangs",
    ExperienceOrb = "ExperienceOrb",
    FallingBlock = "FallingBlock",
    FishingHook = "FishingHook",
    GlowItemFrame = "GlowItemFrame",
    Hanging = "Hanging",
    Interaction = "Interaction",
    ItemFrame = "ItemFrame",
    LeashKnot = "LeashKnot",
    LightningBolt = "LightningBolt",
    LlamaSpit = "LlamaSpit",
    MagmaCube = "MagmaCube",
    Marker = "Marker",
    OminousItemSpawner = "OminousItemSpawner",
    Painting = "Painting",
    PrimedTnt = "PrimedTnt",
    ShulkerBullet = "ShulkerBullet",
    Slime = "Slime",
    TraderLlama = "TraderLlama",
    // projectile
    AbstractArrow = "AbstractArrow",
    AbstractWindCharge = "AbstractWindCharge",
    Arrow = "Arrow",
    BreezeWindCharge = "BreezeWindCharge",
    DragonFireball = "DragonFireball",
    FireworkRocket = "FireworkRocket",
    Projectile = "Projectile",
    SpectralArrow = "SpectralArrow",
    ThrownTrident = "ThrownTrident",
    WindCharge = "WindCharge",
    WitherSkull = "WitherSkull",
    // villager
    AbstractVillager = "AbstractVillager",
    Villager = "Villager",
    WanderingTrader = "WanderingTrader",
    // water
    AgeableWaterAnimal = "AgeableWaterAnimal",
    Axolotl = "Axolotl",
    Dolphin = "Dolphin",
    GlowSquid = "GlowSquid",
    Squid = "Squid",
    WaterAnimal = "WaterAnimal",
    // water.fish
    AbstractFish = "AbstractFish",
    Cod = "Cod",
    Pufferfish = "Pufferfish",
    Salmon = "Salmon",
    Tadpole = "Tadpole",
    TropicalFish = "TropicalFish",
}

// ---------------------------------------------------------------------------
// 互转 trait
// ---------------------------------------------------------------------------

/// 类型化 metadata 与 [`EntityMetadataMap`] 的互转约束。
///
/// 实现方负责把自身字段写成协议 index → [`EntityMetadataValue`] 条目
/// （[`Self::to_map`]），以及从 map 读回（[`Self::from_map`]，字段缺失用
/// 默认值）。
pub trait TypedMeta: Sized {
    /// 自身对应的类别。
    fn meta_type(&self) -> EntityMetaType;
    /// 序列化为元数据表。
    fn to_map(&self) -> EntityMetadataMap;
    /// 从元数据表反序列化（缺失字段用默认值）。
    fn from_map(map: &EntityMetadataMap) -> Self;
}

// ---------------------------------------------------------------------------
// EntityMetadataMap 扩展：与 0x61/0x62 包条目的互转桥梁
// ---------------------------------------------------------------------------

impl EntityMetadataMap {
    /// 导出为包条目（index 收窄到 `u8`；超出 255 的条目跳过）。
    ///
    /// 供 play 层 `entity_metadata`（0x61/0x62）包编码使用。
    pub fn to_entries(&self) -> Vec<(u8, EntityMetadataValue)> {
        self.iter()
            .filter_map(|(index, value)| {
                let index = u8::try_from(*index).ok()?;
                Some((index, value.clone()))
            })
            .collect()
    }

    /// 由包条目（协议 index → 值）构造元数据表。
    pub fn from_entries(entries: &[(u8, EntityMetadataValue)]) -> Self {
        let mut map = EntityMetadataMap::new();
        for (index, value) in entries {
            map.set(u32::from(*index), value.clone());
        }
        map
    }
}

// ---------------------------------------------------------------------------
// map 读字段辅助（字段缺失回默认值；生产代码不用 unwrap/expect）
// ---------------------------------------------------------------------------

fn get_byte(map: &EntityMetadataMap, index: u32) -> u8 {
    match map.get(index) {
        Some(EntityMetadataValue::Byte(v)) => *v,
        _ => 0,
    }
}

fn get_bool(map: &EntityMetadataMap, index: u32) -> bool {
    match map.get(index) {
        Some(EntityMetadataValue::Bool(v)) => *v,
        _ => false,
    }
}

fn get_i32(map: &EntityMetadataMap, index: u32, default: i32) -> i32 {
    match map.get(index) {
        Some(EntityMetadataValue::VarInt(v)) => *v,
        _ => default,
    }
}

fn get_f32(map: &EntityMetadataMap, index: u32, default: f32) -> f32 {
    match map.get(index) {
        Some(EntityMetadataValue::Float(v)) => *v,
        _ => default,
    }
}

// ---------------------------------------------------------------------------
// 手写核心类型
// ---------------------------------------------------------------------------

/// 基础实体 metadata（Java `EntityMeta`，局部 index 0..=7）。
///
/// index 0 为 flags 位掩码：`on_fire(0x01)` / `crouching(0x02)` /
/// `sprinting(0x08)` / `swimming(0x10)` / `invisible(0x20)` / `glowing(0x40)` /
/// `elytra(0x80)`；index 1 = air_ticks（默认 300）；index 2 = custom_name
/// （JSON 文本）；index 3 = custom_name_visible；index 4 = silent；
/// index 5 = no_gravity；index 6 = pose（字节，对齐 `EntityPose` 协议序号）；
/// index 7 = ticks_frozen。
#[derive(Debug, Clone, PartialEq)]
pub struct BaseEntityMeta {
    /// 是否着火（flags 0x01）。
    pub on_fire: bool,
    /// 是否潜行（flags 0x02）。
    pub crouching: bool,
    /// 是否疾跑（flags 0x08）。
    pub sprinting: bool,
    /// 是否游泳（flags 0x10）。
    pub swimming: bool,
    /// 是否隐身（flags 0x20）。
    pub invisible: bool,
    /// 是否发光（flags 0x40）。
    pub glowing: bool,
    /// 是否鞘翅滑翔（flags 0x80）。
    pub elytra: bool,
    /// 空气值（tick，默认 300）。
    pub air_ticks: i32,
    /// 自定义名称（JSON 文本，`None` 表示未设置）。
    pub custom_name: Option<String>,
    /// 自定义名称是否可见。
    pub custom_name_visible: bool,
    /// 是否静音。
    pub silent: bool,
    /// 是否无重力。
    pub no_gravity: bool,
    /// 姿态（协议字节，默认 0 = STANDING）。
    pub pose: u8,
    /// 冻结 tick 数。
    pub ticks_frozen: i32,
}

impl Default for BaseEntityMeta {
    fn default() -> Self {
        Self {
            on_fire: false,
            crouching: false,
            sprinting: false,
            swimming: false,
            invisible: false,
            glowing: false,
            elytra: false,
            air_ticks: 300,
            custom_name: None,
            custom_name_visible: false,
            silent: false,
            no_gravity: false,
            pose: 0,
            ticks_frozen: 0,
        }
    }
}

impl TypedMeta for BaseEntityMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Entity
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut flags: u8 = 0;
        if self.on_fire {
            flags |= 0x01;
        }
        if self.crouching {
            flags |= 0x02;
        }
        if self.sprinting {
            flags |= 0x08;
        }
        if self.swimming {
            flags |= 0x10;
        }
        if self.invisible {
            flags |= 0x20;
        }
        if self.glowing {
            flags |= 0x40;
        }
        if self.elytra {
            flags |= 0x80;
        }
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Byte(flags));
        map.set(1, EntityMetadataValue::VarInt(self.air_ticks));
        if let Some(name) = &self.custom_name {
            map.set(2, EntityMetadataValue::String(name.clone()));
        }
        map.set(3, EntityMetadataValue::Bool(self.custom_name_visible));
        map.set(4, EntityMetadataValue::Bool(self.silent));
        map.set(5, EntityMetadataValue::Bool(self.no_gravity));
        map.set(6, EntityMetadataValue::Byte(self.pose));
        map.set(7, EntityMetadataValue::VarInt(self.ticks_frozen));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        let flags = get_byte(map, 0);
        Self {
            on_fire: flags & 0x01 != 0,
            crouching: flags & 0x02 != 0,
            sprinting: flags & 0x08 != 0,
            swimming: flags & 0x10 != 0,
            invisible: flags & 0x20 != 0,
            glowing: flags & 0x40 != 0,
            elytra: flags & 0x80 != 0,
            air_ticks: get_i32(map, 1, 300),
            custom_name: match map.get(2) {
                Some(EntityMetadataValue::String(v)) => Some(v.clone()),
                _ => None,
            },
            custom_name_visible: get_bool(map, 3),
            silent: get_bool(map, 4),
            no_gravity: get_bool(map, 5),
            pose: get_byte(map, 6),
            ticks_frozen: get_i32(map, 7, 0),
        }
    }
}

/// 生物（living entity）metadata（Java `LivingEntityMeta`，局部 index 0..=6）。
///
/// index 0 为 flags 位掩码：`hand_active(0x01)` / `active_hand_off(0x02)` /
/// `riptide(0x04)`；index 1 = health（默认 1.0）；index 2 = 药水粒子列表
/// （本实现不建模，值类型受限，见 `effect_particles` 说明）；index 3 =
/// ambient；index 4 = arrows；index 5 = bee_stingers；index 6 = 床位置
/// （OptBlockPosition，本实现不建模）。
#[derive(Debug, Clone, PartialEq)]
pub struct LivingEntityMeta {
    /// 是否正在使用手臂（flags 0x01）。
    pub hand_active: bool,
    /// 主手是否 OFF（flags 0x02；false 为主手 MAIN）。
    pub active_hand_off: bool,
    /// 是否三叉戟激流（flags 0x04）。
    pub riptide: bool,
    /// 生命值。
    pub health: f32,
    /// 药水效果是否环境粒子（ambient）。
    pub potion_ambient: bool,
    /// 箭数量。
    pub arrow_count: i32,
    /// 蜜蜂刺数量。
    pub bee_stinger_count: i32,
}

impl Default for LivingEntityMeta {
    fn default() -> Self {
        Self {
            hand_active: false,
            active_hand_off: false,
            riptide: false,
            health: 1.0,
            potion_ambient: false,
            arrow_count: 0,
            bee_stinger_count: 0,
        }
    }
}

impl TypedMeta for LivingEntityMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::LivingEntity
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut flags: u8 = 0;
        if self.hand_active {
            flags |= 0x01;
        }
        if self.active_hand_off {
            flags |= 0x02;
        }
        if self.riptide {
            flags |= 0x04;
        }
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Byte(flags));
        map.set(1, EntityMetadataValue::Float(self.health));
        map.set(3, EntityMetadataValue::Bool(self.potion_ambient));
        map.set(4, EntityMetadataValue::VarInt(self.arrow_count));
        map.set(5, EntityMetadataValue::VarInt(self.bee_stinger_count));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        let flags = get_byte(map, 0);
        Self {
            hand_active: flags & 0x01 != 0,
            active_hand_off: flags & 0x02 != 0,
            riptide: flags & 0x04 != 0,
            health: get_f32(map, 1, 1.0),
            potion_ambient: get_bool(map, 3),
            arrow_count: get_i32(map, 4, 0),
            bee_stinger_count: get_i32(map, 5, 0),
        }
    }
}

/// 普通生物 metadata（Java `MobMeta`）。
///
/// index 0 为 flags 位掩码：`no_ai(0x01)` / `left_handed(0x02)` /
/// `aggressive(0x04)`。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MobMeta {
    /// 是否无 AI（flags 0x01）。
    pub no_ai: bool,
    /// 是否左手（flags 0x02）。
    pub left_handed: bool,
    /// 是否敌对（flags 0x04）。
    pub aggressive: bool,
}

impl TypedMeta for MobMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Mob
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut flags: u8 = 0;
        if self.no_ai {
            flags |= 0x01;
        }
        if self.left_handed {
            flags |= 0x02;
        }
        if self.aggressive {
            flags |= 0x04;
        }
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Byte(flags));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        let flags = get_byte(map, 0);
        Self {
            no_ai: flags & 0x01 != 0,
            left_handed: flags & 0x02 != 0,
            aggressive: flags & 0x04 != 0,
        }
    }
}

/// 可成长动物 metadata（Java `AgeableMobMeta`，任务措辞 `AnimalMeta`）。
///
/// index 0 = is_baby。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AnimalMeta {
    /// 是否为幼体。
    pub is_baby: bool,
}

impl TypedMeta for AnimalMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Animal
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Bool(self.is_baby));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            is_baby: get_bool(map, 0),
        }
    }
}

/// 僵尸 metadata（Java `ZombieMeta`）。
///
/// index 0 = is_baby；index 1 = 未使用（UNUSED，跳过）；index 2 =
/// is_becoming_drowned。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ZombieMeta {
    /// 是否为幼体僵尸。
    pub is_baby: bool,
    /// 是否正在转化为溺尸。
    pub is_becoming_drowned: bool,
}

impl TypedMeta for ZombieMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Zombie
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Bool(self.is_baby));
        map.set(2, EntityMetadataValue::Bool(self.is_becoming_drowned));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            is_baby: get_bool(map, 0),
            is_becoming_drowned: get_bool(map, 2),
        }
    }
}

/// 骷髅 metadata（Java `SkeletonMeta`）。
///
/// 对齐 Java：骷髅及其变种（Stray / WitherSkeleton / Bogged / Parched）没有
/// 自有字段，`is_stray` 不存在（Stray 是独立类别）。本类型仅有抽象层字段，
/// 互转时忽略与骷髅无关的条目。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SkeletonMeta;

impl TypedMeta for SkeletonMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Skeleton
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 苦力怕 metadata（Java `CreeperMeta`）。
///
/// index 0 = state（VarInt：-1 待机 / 1 引信）；index 1 = charged；
/// index 2 = ignited。
#[derive(Debug, Clone, PartialEq)]
pub struct CreeperMeta {
    /// 引信状态（-1 待机，1 引信）。
    pub state: i32,
    /// 是否为闪电苦力怕。
    pub charged: bool,
    /// 是否已被点燃。
    pub ignited: bool,
}

impl Default for CreeperMeta {
    fn default() -> Self {
        Self {
            state: -1,
            charged: false,
            ignited: false,
        }
    }
}

impl TypedMeta for CreeperMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Creeper
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::VarInt(self.state));
        map.set(1, EntityMetadataValue::Bool(self.charged));
        map.set(2, EntityMetadataValue::Bool(self.ignited));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            state: get_i32(map, 0, -1),
            charged: get_bool(map, 1),
            ignited: get_bool(map, 2),
        }
    }
}

/// 猪 metadata（Java `PigMeta`）。
///
/// index 0 = boost_time（加速剩余 tick）。`variant` 在 Java 中已废弃
/// （迁移至 `DataComponents.PIG_VARIANT`），本实现不建模。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PigMeta {
    /// 加速剩余 tick（骑乘时）。
    pub boost_time: i32,
}

impl TypedMeta for PigMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Pig
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::VarInt(self.boost_time));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            boost_time: get_i32(map, 0, 0),
        }
    }
}

/// 狼 metadata（Java `WolfMeta`）。
///
/// index 0 = is_begging；index 1 = collar_color（染料色，默认 14）；
/// index 2 = anger_time（tick，-1 表示未愤怒）。`variant` / `sound_variant`
/// 为注册表 key，本实现不建模（由 [`EntityMetadataMap`] 兜底）。
#[derive(Debug, Clone, PartialEq)]
pub struct WolfMeta {
    /// 是否乞食。
    pub is_begging: bool,
    /// 项圈颜色（染料 id，默认 14 = 红色）。
    pub collar_color: i32,
    /// 愤怒时间（tick，-1 表示未愤怒）。
    pub anger_time: i32,
}

impl Default for WolfMeta {
    fn default() -> Self {
        Self {
            is_begging: false,
            collar_color: 14,
            anger_time: -1,
        }
    }
}

impl TypedMeta for WolfMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Wolf
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Bool(self.is_begging));
        map.set(1, EntityMetadataValue::VarInt(self.collar_color));
        map.set(2, EntityMetadataValue::VarInt(self.anger_time));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            is_begging: get_bool(map, 0),
            collar_color: get_i32(map, 1, 14),
            anger_time: get_i32(map, 2, -1),
        }
    }
}

/// 村民 metadata（Java `VillagerMeta`）。
///
/// index 0 = head_shake_timer（来自 `AbstractVillagerMeta`）；index 1 = 村民
/// 数据（Java 中为复合 `VillagerData`，本实现按三个 VarInt 拆分：
/// type / profession / level；后续 0x62 接入需按复合类型重排）。
#[derive(Debug, Clone, PartialEq)]
pub struct VillagerMeta {
    /// 摇头计时器。
    pub head_shake_timer: i32,
    /// 村民类型 id。
    pub villager_type: i32,
    /// 村民职业 id。
    pub villager_profession: i32,
    /// 村民等级 id（协议 1 起）。
    pub villager_level: i32,
}

impl Default for VillagerMeta {
    fn default() -> Self {
        Self {
            head_shake_timer: 0,
            villager_type: 0,
            villager_profession: 0,
            villager_level: 1,
        }
    }
}

impl TypedMeta for VillagerMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Villager
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::VarInt(self.head_shake_timer));
        map.set(1, EntityMetadataValue::VarInt(self.villager_type));
        map.set(2, EntityMetadataValue::VarInt(self.villager_profession));
        map.set(3, EntityMetadataValue::VarInt(self.villager_level));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            head_shake_timer: get_i32(map, 0, 0),
            villager_type: get_i32(map, 1, 0),
            villager_profession: get_i32(map, 2, 0),
            villager_level: get_i32(map, 3, 1),
        }
    }
}

/// 铁傀儡 metadata（Java `IronGolemMeta`）。
///
/// index 0 为 flags 位掩码：`is_player_created(0x01)`。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct IronGolemMeta {
    /// 是否为玩家创造。
    pub is_player_created: bool,
}

impl TypedMeta for IronGolemMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::IronGolem
    }

    fn to_map(&self) -> EntityMetadataMap {
        let flags: u8 = if self.is_player_created { 0x01 } else { 0 };
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Byte(flags));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            is_player_created: get_byte(map, 0) & 0x01 != 0,
        }
    }
}

/// 史莱姆 metadata（Java `SlimeMeta`）。
///
/// index 0 = size（默认 1）。
#[derive(Debug, Clone, PartialEq)]
pub struct SlimeMeta {
    /// 体型（1 为小、4 为普通、10 为大）。
    pub size: i32,
}

impl Default for SlimeMeta {
    fn default() -> Self {
        Self { size: 1 }
    }
}

impl TypedMeta for SlimeMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Slime
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::VarInt(self.size));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            size: get_i32(map, 0, 1),
        }
    }
}

/// 蝙蝠 metadata（Java `BatMeta`）。
///
/// index 0 为 flags 位掩码：`is_hanging(0x01)`。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BatMeta {
    /// 是否倒挂。
    pub is_hanging: bool,
}

impl TypedMeta for BatMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Bat
    }

    fn to_map(&self) -> EntityMetadataMap {
        let flags: u8 = if self.is_hanging { 0x01 } else { 0 };
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Byte(flags));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            is_hanging: get_byte(map, 0) & 0x01 != 0,
        }
    }
}

/// 钓鱼钩 metadata（Java `FishingHookMeta`）。
///
/// index 0 = hooked（实体 id，0 表示未钩中）；index 1 = is_catchable。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FishingHookMeta {
    /// 钩中的实体 id（0 表示未钩中）。
    pub hooked_entity: i32,
    /// 是否可捕获（咬钩）。
    pub is_catchable: bool,
}

impl TypedMeta for FishingHookMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::FishingHook
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::VarInt(self.hooked_entity));
        map.set(1, EntityMetadataValue::Bool(self.is_catchable));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            hooked_entity: get_i32(map, 0, 0),
            is_catchable: get_bool(map, 1),
        }
    }
}

// ---------------------------------------------------------------------------
// ~40 新增实体 metadata struct（Player / ArmorStand / Display / Flying / Golem /
// Monster / Projectile / Vehicle / 动物 / 其他）
// ---------------------------------------------------------------------------

/// 玩家 metadata（Java `PlayerMeta`）。
///
/// index 0 = additional_hearts；index 1 = score；index 2 = left_shoulder；
/// index 3 = right_shoulder。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlayerMeta {
    /// 额外红心（float）。
    pub additional_hearts: f32,
    /// 分数板分数。
    pub score: i32,
    /// 左肩实体数据（null 表示未设置）。
    pub left_shoulder: Option<i32>,
    /// 右肩实体数据（null 表示未设置）。
    pub right_shoulder: Option<i32>,
}

impl TypedMeta for PlayerMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Player
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Float(self.additional_hearts));
        map.set(1, EntityMetadataValue::VarInt(self.score));
        if let Some(v) = self.left_shoulder {
            map.set(2, EntityMetadataValue::VarInt(v));
        }
        if let Some(v) = self.right_shoulder {
            map.set(3, EntityMetadataValue::VarInt(v));
        }
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            additional_hearts: get_f32(map, 0, 0.0),
            score: get_i32(map, 1, 0),
            left_shoulder: map.get(2).and_then(|v| match v {
                EntityMetadataValue::VarInt(v) => Some(*v),
                _ => None,
            }),
            right_shoulder: map.get(3).and_then(|v| match v {
                EntityMetadataValue::VarInt(v) => Some(*v),
                _ => None,
            }),
        }
    }
}

/// 假人 metadata（Java `MannequinMeta`）。
///
/// index 0 = profile（简化为 String）；1 = immovable；2 = description。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MannequinMeta {
    pub profile: String,
    pub immovable: bool,
    pub description: Option<String>,
}

impl TypedMeta for MannequinMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Mannequin
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::String(self.profile.clone()));
        map.set(1, EntityMetadataValue::Bool(self.immovable));
        if let Some(d) = &self.description {
            map.set(2, EntityMetadataValue::String(d.clone()));
        }
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            profile: match map.get(0) {
                Some(EntityMetadataValue::String(s)) => s.clone(),
                _ => String::new(),
            },
            immovable: get_bool(map, 1),
            description: match map.get(2) {
                Some(EntityMetadataValue::String(s)) => Some(s.clone()),
                _ => None,
            },
        }
    }
}

/// 盔甲架 metadata（Java `ArmorStandMeta`）。
///
/// index 0 = flags（small/arms/no_baseplate/marker）；
/// index 1..=6 = 各部位旋转（x/y/z 各一个 VarInt）。
#[derive(Debug, Clone, PartialEq)]
pub struct ArmorStandMeta {
    pub is_small: bool,
    pub has_arms: bool,
    pub has_no_base_plate: bool,
    pub is_marker: bool,
    pub head_rotation_x: i8,
    pub head_rotation_y: i8,
    pub head_rotation_z: i8,
    pub body_rotation_x: i8,
    pub body_rotation_y: i8,
    pub body_rotation_z: i8,
    pub left_arm_rotation_x: i8,
    pub left_arm_rotation_y: i8,
    pub left_arm_rotation_z: i8,
    pub right_arm_rotation_x: i8,
    pub right_arm_rotation_y: i8,
    pub right_arm_rotation_z: i8,
    pub left_leg_rotation_x: i8,
    pub left_leg_rotation_y: i8,
    pub left_leg_rotation_z: i8,
    pub right_leg_rotation_x: i8,
    pub right_leg_rotation_y: i8,
    pub right_leg_rotation_z: i8,
}

impl Default for ArmorStandMeta {
    fn default() -> Self {
        Self {
            is_small: false,
            has_arms: false,
            has_no_base_plate: false,
            is_marker: false,
            head_rotation_x: 0,
            head_rotation_y: 0,
            head_rotation_z: 0,
            body_rotation_x: 0,
            body_rotation_y: 0,
            body_rotation_z: 0,
            left_arm_rotation_x: -10,
            left_arm_rotation_y: 0,
            left_arm_rotation_z: -10,
            right_arm_rotation_x: -15,
            right_arm_rotation_y: 0,
            right_arm_rotation_z: 10,
            left_leg_rotation_x: -1,
            left_leg_rotation_y: 0,
            left_leg_rotation_z: -1,
            right_leg_rotation_x: 1,
            right_leg_rotation_y: 0,
            right_leg_rotation_z: 1,
        }
    }
}

impl TypedMeta for ArmorStandMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::ArmorStand
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut flags: u8 = 0;
        if self.is_small {
            flags |= 0x01;
        }
        if self.has_arms {
            flags |= 0x04;
        }
        if self.has_no_base_plate {
            flags |= 0x08;
        }
        if self.is_marker {
            flags |= 0x10;
        }
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Byte(flags));
        macro_rules! set_rot {
            ($m:ident, $base:expr, $x:expr, $y:expr, $z:expr) => {
                $m.set($base, EntityMetadataValue::VarInt($x as i32));
                $m.set($base + 1, EntityMetadataValue::VarInt($y as i32));
                $m.set($base + 2, EntityMetadataValue::VarInt($z as i32));
            };
        }
        set_rot!(
            map,
            1,
            self.head_rotation_x,
            self.head_rotation_y,
            self.head_rotation_z
        );
        set_rot!(
            map,
            4,
            self.body_rotation_x,
            self.body_rotation_y,
            self.body_rotation_z
        );
        set_rot!(
            map,
            7,
            self.left_arm_rotation_x,
            self.left_arm_rotation_y,
            self.left_arm_rotation_z
        );
        set_rot!(
            map,
            10,
            self.right_arm_rotation_x,
            self.right_arm_rotation_y,
            self.right_arm_rotation_z
        );
        set_rot!(
            map,
            13,
            self.left_leg_rotation_x,
            self.left_leg_rotation_y,
            self.left_leg_rotation_z
        );
        set_rot!(
            map,
            16,
            self.right_leg_rotation_x,
            self.right_leg_rotation_y,
            self.right_leg_rotation_z
        );
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        let flags = get_byte(map, 0);
        macro_rules! get_rot {
            ($m:ident, $base:expr) => {{
                (
                    $m.get($base)
                        .and_then(|v| match v {
                            EntityMetadataValue::VarInt(v) => Some(*v as i8),
                            _ => None,
                        })
                        .unwrap_or(0),
                    $m.get($base + 1)
                        .and_then(|v| match v {
                            EntityMetadataValue::VarInt(v) => Some(*v as i8),
                            _ => None,
                        })
                        .unwrap_or(0),
                    $m.get($base + 2)
                        .and_then(|v| match v {
                            EntityMetadataValue::VarInt(v) => Some(*v as i8),
                            _ => None,
                        })
                        .unwrap_or(0),
                )
            }};
        }
        let (hx, hy, hz) = get_rot!(map, 1);
        let (bx, by, bz) = get_rot!(map, 4);
        let (lax, lay, laz) = get_rot!(map, 7);
        let (rax, ray, raz) = get_rot!(map, 10);
        let (llx, lly, llz) = get_rot!(map, 13);
        let (rlx, rly, rlz) = get_rot!(map, 16);
        Self {
            is_small: flags & 0x01 != 0,
            has_arms: flags & 0x04 != 0,
            has_no_base_plate: flags & 0x08 != 0,
            is_marker: flags & 0x10 != 0,
            head_rotation_x: hx,
            head_rotation_y: hy,
            head_rotation_z: hz,
            body_rotation_x: bx,
            body_rotation_y: by,
            body_rotation_z: bz,
            left_arm_rotation_x: lax,
            left_arm_rotation_y: lay,
            left_arm_rotation_z: laz,
            right_arm_rotation_x: rax,
            right_arm_rotation_y: ray,
            right_arm_rotation_z: raz,
            left_leg_rotation_x: llx,
            left_leg_rotation_y: lly,
            left_leg_rotation_z: llz,
            right_leg_rotation_x: rlx,
            right_leg_rotation_y: rly,
            right_leg_rotation_z: rlz,
        }
    }
}

// ---- Display entities ----

/// 展示实体抽象基类 metadata（Java `AbstractDisplayMeta`）。
///
/// index 0 = interpolation_delay；1 = transformation_duration；
/// 2 = pos_rot_duration；3 = brightness_override；4 = view_range；
/// 5 = shadow_radius；6 = shadow_strength；7 = width；8 = height；
/// 9 = glow_color_override。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AbstractDisplayMeta {
    pub interpolation_delay: i32,
    pub transformation_interpolation_duration: i32,
    pub pos_rot_interpolation_duration: i32,
    pub brightness_override: i32,
    pub view_range: f32,
    pub shadow_radius: f32,
    pub shadow_strength: f32,
    pub width: f32,
    pub height: f32,
    pub glow_color_override: i32,
}

impl TypedMeta for AbstractDisplayMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::AbstractDisplay
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::VarInt(self.interpolation_delay));
        map.set(
            1,
            EntityMetadataValue::VarInt(self.transformation_interpolation_duration),
        );
        map.set(
            2,
            EntityMetadataValue::VarInt(self.pos_rot_interpolation_duration),
        );
        map.set(3, EntityMetadataValue::VarInt(self.brightness_override));
        map.set(4, EntityMetadataValue::Float(self.view_range));
        map.set(5, EntityMetadataValue::Float(self.shadow_radius));
        map.set(6, EntityMetadataValue::Float(self.shadow_strength));
        map.set(7, EntityMetadataValue::Float(self.width));
        map.set(8, EntityMetadataValue::Float(self.height));
        map.set(9, EntityMetadataValue::VarInt(self.glow_color_override));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            interpolation_delay: get_i32(map, 0, 0),
            transformation_interpolation_duration: get_i32(map, 1, 0),
            pos_rot_interpolation_duration: get_i32(map, 2, 0),
            brightness_override: get_i32(map, 3, -1),
            view_range: get_f32(map, 4, 1.0),
            shadow_radius: get_f32(map, 5, 0.0),
            shadow_strength: get_f32(map, 6, 1.0),
            width: get_f32(map, 7, 0.0),
            height: get_f32(map, 8, 0.0),
            glow_color_override: get_i32(map, 9, -1),
        }
    }
}

/// 方块展示实体 metadata（Java `BlockDisplayMeta`）。
///
/// index 0 = block_state_id。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BlockDisplayMeta {
    pub displayed_block_state: i32,
}

impl TypedMeta for BlockDisplayMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::BlockDisplay
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::VarInt(self.displayed_block_state));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            displayed_block_state: get_i32(map, 0, 0),
        }
    }
}

/// 物品展示实体 metadata（Java `ItemDisplayMeta`）。
///
/// index 0 = item_stack_id；1 = display_context。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ItemDisplayMeta {
    pub displayed_item: i32,
    pub display_context: u8,
}

impl TypedMeta for ItemDisplayMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::ItemDisplay
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::VarInt(self.displayed_item));
        map.set(1, EntityMetadataValue::Byte(self.display_context));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            displayed_item: get_i32(map, 0, 0),
            display_context: get_byte(map, 1),
        }
    }
}

/// 文字展示实体 metadata（Java `TextDisplayMeta`）。
///
/// index 0 = text；1 = line_width；2 = background_color；
/// 3 = text_opacity；4 = flags。
#[derive(Debug, Clone, PartialEq)]
pub struct TextDisplayMeta {
    pub text: String,
    pub line_width: i32,
    pub background_color: i32,
    pub text_opacity: i8,
    pub flags: u8,
}

impl Default for TextDisplayMeta {
    fn default() -> Self {
        Self {
            text: String::new(),
            line_width: 200,
            background_color: 0x40000000,
            text_opacity: -1,
            flags: 0,
        }
    }
}

impl TypedMeta for TextDisplayMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::TextDisplay
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::String(self.text.clone()));
        map.set(1, EntityMetadataValue::VarInt(self.line_width));
        map.set(2, EntityMetadataValue::VarInt(self.background_color));
        map.set(3, EntityMetadataValue::Byte(self.text_opacity as u8));
        map.set(4, EntityMetadataValue::Byte(self.flags));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            text: match map.get(0) {
                Some(EntityMetadataValue::String(s)) => s.clone(),
                _ => String::new(),
            },
            line_width: get_i32(map, 1, 200),
            background_color: get_i32(map, 2, 0x40000000),
            text_opacity: map
                .get(3)
                .and_then(|v| match v {
                    EntityMetadataValue::Byte(b) => Some(*b as i8),
                    _ => None,
                })
                .unwrap_or(-1),
            flags: get_byte(map, 4),
        }
    }
}

// ---- Flying entities ----

/// 飞行实体抽象基类 metadata（Java `FlyingMeta`）。无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FlyingMeta;

impl TypedMeta for FlyingMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Flying
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 恶魂 metadata（Java `GhastMeta`）。
///
/// index 0 = attacking。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GhastMeta {
    pub attacking: bool,
}

impl TypedMeta for GhastMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Ghast
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Bool(self.attacking));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            attacking: get_bool(map, 0),
        }
    }
}

/// 幻翼 metadata（Java `PhantomMeta`）。
///
/// index 0 = size。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PhantomMeta {
    pub size: i32,
}

impl TypedMeta for PhantomMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Phantom
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::VarInt(self.size));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            size: get_i32(map, 0, 0),
        }
    }
}

// ---- Golems ----

/// 雪人傀儡 metadata（Java `SnowGolemMeta`）。
///
/// index 0 = pumpkin_hat（flags 0x10）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SnowGolemMeta {
    pub pumpkin_hat: bool,
}

impl TypedMeta for SnowGolemMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::SnowGolem
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        let flags: u8 = if self.pumpkin_hat { 0x10 } else { 0 };
        map.set(0, EntityMetadataValue::Byte(flags));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            pumpkin_hat: get_byte(map, 0) & 0x10 != 0,
        }
    }
}

/// 潜影贝 metadata（Java `ShulkerMeta`）。
///
/// index 0 = attach_face；1 = shield_height；2 = color。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ShulkerMeta {
    pub attach_face: u8,
    pub shield_height: u8,
    pub color: u8,
}

impl TypedMeta for ShulkerMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Shulker
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Byte(self.attach_face));
        map.set(1, EntityMetadataValue::Byte(self.shield_height));
        map.set(2, EntityMetadataValue::Byte(self.color));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            attach_face: get_byte(map, 0),
            shield_height: get_byte(map, 1),
            color: get_byte(map, 2),
        }
    }
}

/// 铜傀儡 metadata（Java `CopperGolemMeta`）。
///
/// index 0 = weather_state；1 = state。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CopperGolemMeta {
    pub weather_state: u8,
    pub state: u8,
}

impl TypedMeta for CopperGolemMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::CopperGolem
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Byte(self.weather_state));
        map.set(1, EntityMetadataValue::Byte(self.state));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            weather_state: get_byte(map, 0),
            state: get_byte(map, 1),
        }
    }
}

// ---- Monsters ----

/// 蜘蛛 metadata（Java `SpiderMeta`）。
///
/// index 0 = flags（climbing 0x01）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpiderMeta {
    pub climbing: bool,
}

impl TypedMeta for SpiderMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Spider
    }

    fn to_map(&self) -> EntityMetadataMap {
        let flags: u8 = if self.climbing { 0x01 } else { 0 };
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Byte(flags));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            climbing: get_byte(map, 0) & 0x01 != 0,
        }
    }
}

/// 烈焰人 metadata（Java `BlazeMeta`）。
///
/// index 0 = flags（on_fire 0x01）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BlazeMeta {
    pub on_fire: bool,
}

impl TypedMeta for BlazeMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Blaze
    }

    fn to_map(&self) -> EntityMetadataMap {
        let flags: u8 = if self.on_fire { 0x01 } else { 0 };
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Byte(flags));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            on_fire: get_byte(map, 0) & 0x01 != 0,
        }
    }
}

/// 末影人 metadata（Java `EndermanMeta`）。
///
/// index 0 = carried_block；1 = screaming；2 = staring。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EndermanMeta {
    pub carried_block: i32,
    pub screaming: bool,
    pub staring: bool,
}

impl TypedMeta for EndermanMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Enderman
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::VarInt(self.carried_block));
        map.set(1, EntityMetadataValue::Bool(self.screaming));
        map.set(2, EntityMetadataValue::Bool(self.staring));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            carried_block: get_i32(map, 0, -1),
            screaming: get_bool(map, 1),
            staring: get_bool(map, 2),
        }
    }
}

/// 监守者 metadata（Java `WardenMeta`）。
///
/// index 0 = anger_level。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WardenMeta {
    pub anger_level: i32,
}

impl TypedMeta for WardenMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Warden
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::VarInt(self.anger_level));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            anger_level: get_i32(map, 0, 0),
        }
    }
}

/// 凋灵 metadata（Java `WitherMeta`）。
///
/// index 0 = center_head；1 = left_head；2 = right_head；3 = invulnerable_time。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WitherMeta {
    pub center_head_target: i32,
    pub left_head_target: i32,
    pub right_head_target: i32,
    pub invulnerable_time: i32,
}

impl TypedMeta for WitherMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Wither
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::VarInt(self.center_head_target));
        map.set(1, EntityMetadataValue::VarInt(self.left_head_target));
        map.set(2, EntityMetadataValue::VarInt(self.right_head_target));
        map.set(3, EntityMetadataValue::VarInt(self.invulnerable_time));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            center_head_target: get_i32(map, 0, 0),
            left_head_target: get_i32(map, 1, 0),
            right_head_target: get_i32(map, 2, 0),
            invulnerable_time: get_i32(map, 3, 0),
        }
    }
}

/// 岩浆怪 metadata 无自有字段（同 SlimeMeta size）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MagmaCubeMeta;

impl TypedMeta for MagmaCubeMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::MagmaCube
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 恼鬼 metadata（Java `VexMeta`）。
///
/// index 0 = flags（attacking 0x01）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct VexMeta {
    pub attacking: bool,
}

impl TypedMeta for VexMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Vex
    }

    fn to_map(&self) -> EntityMetadataMap {
        let flags: u8 = if self.attacking { 0x01 } else { 0 };
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Byte(flags));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            attacking: get_byte(map, 0) & 0x01 != 0,
        }
    }
}

/// 猪布林 metadata（Java `PiglinMeta`）。
///
/// index 0 = is_baby；1 = charging_crossbow；2 = dancing。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PiglinMeta {
    pub is_baby: bool,
    pub charging_crossbow: bool,
    pub dancing: bool,
}

impl TypedMeta for PiglinMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Piglin
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Bool(self.is_baby));
        map.set(1, EntityMetadataValue::Bool(self.charging_crossbow));
        map.set(2, EntityMetadataValue::Bool(self.dancing));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            is_baby: get_bool(map, 0),
            charging_crossbow: get_bool(map, 1),
            dancing: get_bool(map, 2),
        }
    }
}

/// 女巫 metadata（Java `WitchMeta`）。
///
/// index 0 = drinking_potion。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WitchMeta {
    pub drinking_potion: bool,
}

impl TypedMeta for WitchMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Witch
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Bool(self.drinking_potion));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            drinking_potion: get_bool(map, 0),
        }
    }
}

/// 召唤者 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EvokerMeta;

impl TypedMeta for EvokerMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Evoker
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 伪装者 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct IllusionerMeta;

impl TypedMeta for IllusionerMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Illusioner
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 蝾螈 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RavagerMeta;

impl TypedMeta for RavagerMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Ravager
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 卫道士 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct VindicatorMeta;

impl TypedMeta for VindicatorMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Vindicator
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 灾厄村民抽象基类 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AbstractIllagerMeta;

impl TypedMeta for AbstractIllagerMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::AbstractIllager
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 劫掠者抽象基类 metadata（Java `RaiderMeta`）。
///
/// index 0 = celebrating。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RaiderMeta {
    pub celebrating: bool,
}

impl TypedMeta for RaiderMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Raider
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Bool(self.celebrating));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            celebrating: get_bool(map, 0),
        }
    }
}

/// 施法者Illager metadata（Java `SpellcasterIllagerMeta`）。
///
/// index 0 = spell（0=NONE,…,5=BLINDNESS）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpellcasterIllagerMeta {
    pub spell: u8,
}

impl TypedMeta for SpellcasterIllagerMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::SpellcasterIllager
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Byte(self.spell));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            spell: get_byte(map, 0),
        }
    }
}

/// 掠夺者 metadata（Java `PillagerMeta`）。
///
/// index 0 = charging_crossbow。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PillagerMeta {
    pub charging_crossbow: bool,
}

impl TypedMeta for PillagerMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Pillager
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Bool(self.charging_crossbow));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            charging_crossbow: get_bool(map, 0),
        }
    }
}

/// 蠕虫 metadata（Java `CreakingMeta`）。
///
/// index 0 = can_move；1 = is_active；2 = is_tearing_down；3 = home_pos_x。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CreakingMeta {
    pub can_move: bool,
    pub is_active: bool,
    pub is_tearing_down: bool,
    pub home_pos_x: i32,
}

impl TypedMeta for CreakingMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Creaking
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Bool(self.can_move));
        map.set(1, EntityMetadataValue::Bool(self.is_active));
        map.set(2, EntityMetadataValue::Bool(self.is_tearing_down));
        map.set(3, EntityMetadataValue::VarInt(self.home_pos_x));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            can_move: get_bool(map, 0),
            is_active: get_bool(map, 1),
            is_tearing_down: get_bool(map, 2),
            home_pos_x: get_i32(map, 3, 0),
        }
    }
}

/// 僵尸村民 metadata（Java `ZombieVillagerMeta`）。
///
/// index 0 = is_converting。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ZombieVillagerMeta {
    pub is_converting: bool,
}

impl TypedMeta for ZombieVillagerMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::ZombieVillager
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Bool(self.is_converting));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            is_converting: get_bool(map, 0),
        }
    }
}

/// 沼泽僵尸 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HuskMeta;

impl TypedMeta for HuskMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Husk
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 溺尸 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DrownedMeta;

impl TypedMeta for DrownedMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Drowned
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 僵尸猪灵 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ZombifiedPiglinMeta;

impl TypedMeta for ZombifiedPiglinMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::ZombifiedPiglin
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 猪布林蛮兵 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PiglinBruteMeta;

impl TypedMeta for PiglinBruteMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::PiglinBrute
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 远古守卫者 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ElderGuardianMeta;

impl TypedMeta for ElderGuardianMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::ElderGuardian
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 守卫者 metadata（Java `GuardianMeta`）。
///
/// index 0 = retracting_spikes；1 = target_entity_id。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GuardianMeta {
    pub retracting_spikes: bool,
    pub target_entity_id: i32,
}

impl TypedMeta for GuardianMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Guardian
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Bool(self.retracting_spikes));
        map.set(1, EntityMetadataValue::VarInt(self.target_entity_id));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            retracting_spikes: get_bool(map, 0),
            target_entity_id: get_i32(map, 1, 0),
        }
    }
}

/// 巨人 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GiantMeta;

impl TypedMeta for GiantMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Giant
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 爬虫 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SilverfishMeta;

impl TypedMeta for SilverfishMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Silverfish
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 洞穴蜘蛛 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CaveSpiderMeta;

impl TypedMeta for CaveSpiderMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::CaveSpider
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 末影螨 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EndermiteMeta;

impl TypedMeta for EndermiteMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Endermite
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 旋风 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BreezeMeta;

impl TypedMeta for BreezeMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Breeze
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 焦骨骷髅 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StrayMeta;

impl TypedMeta for StrayMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Stray
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 枯萎骷髅 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WitherSkeletonMeta;

impl TypedMeta for WitherSkeletonMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::WitherSkeleton
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 沙漠骷髅 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParchedMeta;

impl TypedMeta for ParchedMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Parched
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 泥泞骷髅 metadata（Java `BoggedMeta`）。
///
/// index 0 = is_sheared。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BoggedMeta {
    pub is_sheared: bool,
}

impl TypedMeta for BoggedMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Bogged
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Bool(self.is_sheared));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            is_sheared: get_bool(map, 0),
        }
    }
}

/// 基岩猪布林 metadata（Java `BasePiglinMeta`）。
///
/// index 0 = immune_to_zombification。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BasePiglinMeta {
    pub immune_to_zombification: bool,
}

impl TypedMeta for BasePiglinMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::BasePiglin
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Bool(self.immune_to_zombification));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            immune_to_zombification: get_bool(map, 0),
        }
    }
}

// ---- Projectiles ----

/// 箭抽象基类 metadata（Java `AbstractArrowMeta`）。
///
/// index 0 = flags（critical 0x01 / no_clip 0x02）；
/// index 1 = piercing_level；index 2 = in_ground。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AbstractArrowMeta {
    pub is_critical: bool,
    pub is_no_clip: bool,
    pub piercing_level: u8,
    pub is_in_ground: bool,
}

impl TypedMeta for AbstractArrowMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::AbstractArrow
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut flags: u8 = 0;
        if self.is_critical {
            flags |= 0x01;
        }
        if self.is_no_clip {
            flags |= 0x02;
        }
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Byte(flags));
        map.set(1, EntityMetadataValue::Byte(self.piercing_level));
        map.set(2, EntityMetadataValue::Bool(self.is_in_ground));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        let flags = get_byte(map, 0);
        Self {
            is_critical: flags & 0x01 != 0,
            is_no_clip: flags & 0x02 != 0,
            piercing_level: get_byte(map, 1),
            is_in_ground: get_bool(map, 2),
        }
    }
}

/// 普通箭 metadata（Java `ArrowMeta`）。
///
/// index 3 = color（-1=默认）。
#[derive(Debug, Clone, PartialEq)]
pub struct ArrowMeta {
    pub color: i32,
}

impl Default for ArrowMeta {
    fn default() -> Self {
        Self { color: -1 }
    }
}

impl TypedMeta for ArrowMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Arrow
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(3, EntityMetadataValue::VarInt(self.color));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            color: get_i32(map, 3, -1),
        }
    }
}

/// 闪烁箭 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpectralArrowMeta;

impl TypedMeta for SpectralArrowMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::SpectralArrow
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 掷出三叉戟 metadata（Java `ThrownTridentMeta`）。
///
/// index 3 = loyalty_level；4 = has_enchantment_glint。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ThrownTridentMeta {
    pub loyalty_level: u8,
    pub has_enchantment_glint: bool,
}

impl TypedMeta for ThrownTridentMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::ThrownTrident
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(3, EntityMetadataValue::Byte(self.loyalty_level));
        map.set(4, EntityMetadataValue::Bool(self.has_enchantment_glint));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            loyalty_level: get_byte(map, 3),
            has_enchantment_glint: get_bool(map, 4),
        }
    }
}

/// 凋灵骷髅头颅 metadata（Java `WitherSkullMeta`）。
///
/// index 0 = invulnerable。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WitherSkullMeta {
    pub invulnerable: bool,
}

impl TypedMeta for WitherSkullMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::WitherSkull
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Bool(self.invulnerable));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            invulnerable: get_bool(map, 0),
        }
    }
}

/// 末影龙火球 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DragonFireballMeta;

impl TypedMeta for DragonFireballMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::DragonFireball
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 火球 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FireballMeta;

impl TypedMeta for FireballMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Fireball
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 小火焰弹 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SmallFireballMeta;

impl TypedMeta for SmallFireballMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::SmallFireball
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 雪球 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SnowballMeta;

impl TypedMeta for SnowballMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Snowball
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 投掷鸡蛋 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ThrownEggMeta;

impl TypedMeta for ThrownEggMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::ThrownEgg
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 投掷末影珍珠 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ThrownEnderPearlMeta;

impl TypedMeta for ThrownEnderPearlMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::ThrownEnderPearl
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 投掷经验瓶 metadata（Java `ThrownExperienceBottleMeta`）。
///
/// index 0 = value。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ThrownExperienceBottleMeta {
    pub value: i32,
}

impl TypedMeta for ThrownExperienceBottleMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::ThrownExperienceBottle
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::VarInt(self.value));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            value: get_i32(map, 0, 0),
        }
    }
}

/// 喷雾药水 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SplashPotionMeta;

impl TypedMeta for SplashPotionMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::SplashPotion
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 滞留药水 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LingeringPotionMeta;

impl TypedMeta for LingeringPotionMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::LingeringPotion
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 末影之眼 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EyeOfEnderMeta;

impl TypedMeta for EyeOfEnderMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::EyeOfEnder
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

// ---- Vehicles ----

/// 矿车抽象基类 metadata（Java `AbstractMinecartMeta`）。
///
/// index 2 = custom_block_state；3 = custom_block_y。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AbstractMinecartMeta {
    pub custom_block_state: i32,
    pub custom_block_y: i32,
}

impl TypedMeta for AbstractMinecartMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::AbstractMinecart
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(2, EntityMetadataValue::VarInt(self.custom_block_state));
        map.set(3, EntityMetadataValue::VarInt(self.custom_block_y));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            custom_block_state: get_i32(map, 2, -1),
            custom_block_y: get_i32(map, 3, 6),
        }
    }
}

/// 普通矿车 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MinecartMeta;

impl TypedMeta for MinecartMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Minecart
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 箱子矿车 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChestMinecartMeta;

impl TypedMeta for ChestMinecartMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::ChestMinecart
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 漏斗矿车 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HopperMinecartMeta;

impl TypedMeta for HopperMinecartMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::HopperMinecart
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// TNT矿车 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TntMinecartMeta;

impl TypedMeta for TntMinecartMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::TntMinecart
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 刷怪笼矿车 metadata 无自有字段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpawnerMinecartMeta;

impl TypedMeta for SpawnerMinecartMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::SpawnerMinecart
    }

    fn to_map(&self) -> EntityMetadataMap {
        EntityMetadataMap::new()
    }

    fn from_map(_map: &EntityMetadataMap) -> Self {
        Self
    }
}

/// 熔炉矿车 metadata（Java `FurnaceMinecartMeta`）。
///
/// index 2 = has_fuel。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FurnaceMinecartMeta {
    pub has_fuel: bool,
}

impl TypedMeta for FurnaceMinecartMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::FurnaceMinecart
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(2, EntityMetadataValue::Bool(self.has_fuel));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            has_fuel: get_bool(map, 2),
        }
    }
}

/// 命令块矿车 metadata（Java `CommandBlockMinecartMeta`）。
///
/// index 2 = command；3 = last_output。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CommandBlockMinecartMeta {
    pub command: String,
    pub last_output: Option<String>,
}

impl TypedMeta for CommandBlockMinecartMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::CommandBlockMinecart
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(2, EntityMetadataValue::String(self.command.clone()));
        if let Some(output) = &self.last_output {
            map.set(3, EntityMetadataValue::String(output.clone()));
        }
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            command: match map.get(2) {
                Some(EntityMetadataValue::String(s)) => s.clone(),
                _ => String::new(),
            },
            last_output: match map.get(3) {
                Some(EntityMetadataValue::String(s)) => Some(s.clone()),
                _ => None,
            },
        }
    }
}

/// 船 metadata（Java `BoatMeta`）。
///
/// index 3 = left_paddle_turning；4 = right_paddle_turning；5 = splash_timer。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BoatMeta {
    pub left_paddle_turning: bool,
    pub right_paddle_turning: bool,
    pub splash_timer: i32,
}

impl TypedMeta for BoatMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Boat
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(3, EntityMetadataValue::Bool(self.left_paddle_turning));
        map.set(4, EntityMetadataValue::Bool(self.right_paddle_turning));
        map.set(5, EntityMetadataValue::VarInt(self.splash_timer));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            left_paddle_turning: get_bool(map, 3),
            right_paddle_turning: get_bool(map, 4),
            splash_timer: get_i32(map, 5, 0),
        }
    }
}

// ---- Animals & other mobs ----

/// 悦灵 metadata（Java `AllayMeta`）。
///
/// index 0 = dancing；1 = can_duplicate。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AllayMeta {
    pub dancing: bool,
    pub can_duplicate: bool,
}

impl TypedMeta for AllayMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Allay
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Bool(self.dancing));
        map.set(1, EntityMetadataValue::Bool(self.can_duplicate));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            dancing: get_bool(map, 0),
            can_duplicate: get_bool(map, 1),
        }
    }
}

/// 经验球 metadata（Java `ExperienceOrbMeta`）。
///
/// index 0 = value。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExperienceOrbMeta {
    pub value: i32,
}

impl TypedMeta for ExperienceOrbMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::ExperienceOrb
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::VarInt(self.value));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            value: get_i32(map, 0, 0),
        }
    }
}

/// 区域效果云 metadata（Java `AreaEffectCloudMeta`）。
///
/// index 0 = radius；1 = waiting；2 = particle_id。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AreaEffectCloudMeta {
    pub radius: f32,
    pub waiting: bool,
    pub particle_id: i32,
}

impl TypedMeta for AreaEffectCloudMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::AreaEffectCloud
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Float(self.radius));
        map.set(1, EntityMetadataValue::Bool(self.waiting));
        map.set(2, EntityMetadataValue::VarInt(self.particle_id));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            radius: get_f32(map, 0, 0.5),
            waiting: get_bool(map, 1),
            particle_id: get_i32(map, 2, 0),
        }
    }
}

/// 狐狸 metadata（Java `FoxMeta`）。
///
/// index 1 = flags（sitting 0x01 / crouching 0x04 / interested 0x08 /
/// pouncing 0x10 / sleeping 0x20 / faceplanted 0x40 / defending 0x80）；
/// index 2 = first_uuid_low；3 = second_uuid_low。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FoxMeta {
    pub sitting: bool,
    pub crouching: bool,
    pub interested: bool,
    pub pouncing: bool,
    pub sleeping: bool,
    pub faceplanted: bool,
    pub defending: bool,
    pub first_uuid_low: i32,
    pub second_uuid_low: i32,
}

impl TypedMeta for FoxMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Fox
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut flags: u8 = 0;
        if self.sitting {
            flags |= 0x01;
        }
        if self.crouching {
            flags |= 0x04;
        }
        if self.interested {
            flags |= 0x08;
        }
        if self.pouncing {
            flags |= 0x10;
        }
        if self.sleeping {
            flags |= 0x20;
        }
        if self.faceplanted {
            flags |= 0x40;
        }
        if self.defending {
            flags |= 0x80;
        }
        let mut map = EntityMetadataMap::new();
        map.set(1, EntityMetadataValue::Byte(flags));
        map.set(2, EntityMetadataValue::VarInt(self.first_uuid_low));
        map.set(3, EntityMetadataValue::VarInt(self.second_uuid_low));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        let flags = get_byte(map, 1);
        Self {
            sitting: flags & 0x01 != 0,
            crouching: flags & 0x04 != 0,
            interested: flags & 0x08 != 0,
            pouncing: flags & 0x10 != 0,
            sleeping: flags & 0x20 != 0,
            faceplanted: flags & 0x40 != 0,
            defending: flags & 0x80 != 0,
            first_uuid_low: get_i32(map, 2, 0),
            second_uuid_low: get_i32(map, 3, 0),
        }
    }
}

/// 嗅探兽 metadata（Java `SnifferMeta`）。
///
/// index 0 = state（0=IDLING,…,5=RISING）；1 = drop_seed_at_tick。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SnifferMeta {
    pub state: u8,
    pub drop_seed_at_tick: i32,
}

impl TypedMeta for SnifferMeta {
    fn meta_type(&self) -> EntityMetaType {
        EntityMetaType::Sniffer
    }

    fn to_map(&self) -> EntityMetadataMap {
        let mut map = EntityMetadataMap::new();
        map.set(0, EntityMetadataValue::Byte(self.state));
        map.set(1, EntityMetadataValue::VarInt(self.drop_seed_at_tick));
        map
    }

    fn from_map(map: &EntityMetadataMap) -> Self {
        Self {
            state: get_byte(map, 0),
            drop_seed_at_tick: get_i32(map, 1, 0),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// 抽查 roundtrip：`to_map` → `from_map` 还原。
    #[test]
    fn zombie_roundtrip() {
        let meta = ZombieMeta {
            is_baby: true,
            is_becoming_drowned: true,
        };
        let map = meta.to_map();
        let restored = ZombieMeta::from_map(&map);
        assert_eq!(restored, meta);
    }

    #[test]
    fn creeper_roundtrip() {
        let meta = CreeperMeta {
            state: 1,
            charged: true,
            ignited: true,
        };
        let restored = CreeperMeta::from_map(&meta.to_map());
        assert_eq!(restored, meta);
    }

    #[test]
    fn player_adjacent_roundtrips() {
        // 玩家相关链：BaseEntityMeta（玩家共用基础字段）。
        let base = BaseEntityMeta {
            on_fire: true,
            sprinting: true,
            custom_name: Some("{\"text\":\"Hi\"}".to_string()),
            ..BaseEntityMeta::default()
        };
        let restored = BaseEntityMeta::from_map(&base.to_map());
        assert_eq!(restored, base);
        // LivingEntityMeta。
        let living = LivingEntityMeta {
            health: 20.0,
            arrow_count: 3,
            ..LivingEntityMeta::default()
        };
        assert_eq!(LivingEntityMeta::from_map(&living.to_map()), living);
        // AnimalMeta / WolfMeta / VillagerMeta / IronGolemMeta / SlimeMeta /
        // BatMeta / FishingHookMeta / PigMeta。
        assert_eq!(
            AnimalMeta::from_map(&AnimalMeta { is_baby: true }.to_map()),
            AnimalMeta { is_baby: true }
        );
        let wolf = WolfMeta {
            is_begging: true,
            collar_color: 3,
            anger_time: 100,
        };
        assert_eq!(WolfMeta::from_map(&wolf.to_map()), wolf);
        let villager = VillagerMeta {
            head_shake_timer: 5,
            villager_type: 1,
            villager_profession: 2,
            villager_level: 3,
        };
        assert_eq!(VillagerMeta::from_map(&villager.to_map()), villager);
        let golem = IronGolemMeta {
            is_player_created: true,
        };
        assert_eq!(IronGolemMeta::from_map(&golem.to_map()), golem);
        assert_eq!(
            SlimeMeta::from_map(&SlimeMeta { size: 4 }.to_map()),
            SlimeMeta { size: 4 }
        );
        assert_eq!(
            BatMeta::from_map(&BatMeta { is_hanging: true }.to_map()),
            BatMeta { is_hanging: true }
        );
        assert_eq!(
            FishingHookMeta::from_map(
                &FishingHookMeta {
                    hooked_entity: 42,
                    is_catchable: true
                }
                .to_map()
            ),
            FishingHookMeta {
                hooked_entity: 42,
                is_catchable: true
            }
        );
        assert_eq!(
            PigMeta::from_map(&PigMeta { boost_time: 7 }.to_map()),
            PigMeta { boost_time: 7 }
        );
        // SkeletonMeta：无自有字段。
        assert_eq!(SkeletonMeta::from_map(&SkeletonMeta.to_map()), SkeletonMeta);
    }

    /// map 互转：字段写出到预期 index，缺失字段回默认值。
    #[test]
    fn map_interop() {
        let zombie = ZombieMeta {
            is_baby: true,
            is_becoming_drowned: false,
        };
        let map = zombie.to_map();
        assert_eq!(map.get(0), Some(&EntityMetadataValue::Bool(true)));
        assert_eq!(map.get(2), Some(&EntityMetadataValue::Bool(false)));
        // 从空表还原 → 默认值。
        assert_eq!(
            ZombieMeta::from_map(&EntityMetadataMap::new()),
            ZombieMeta::default()
        );
        // Creeper state 默认 -1。
        assert_eq!(
            CreeperMeta::from_map(&EntityMetadataMap::new()),
            CreeperMeta::default()
        );
    }

    /// from_name 全表：注册表中每个类名都可解析且可往返。
    #[test]
    fn from_name_full_table() {
        for ty in EntityMetaType::all() {
            let name = ty.as_str();
            assert!(name.ends_with("Meta"), "类名应以 Meta 结尾: {name}");
            assert_eq!(EntityMetaType::from_name(name), Some(*ty), "类名 {name}");
            // 基名也可解析。
            assert_eq!(
                EntityMetaType::from_name(name.trim_end_matches("Meta")),
                Some(*ty)
            );
        }
    }

    #[test]
    fn from_name_unknown_returns_none() {
        assert_eq!(EntityMetaType::from_name("NotAThing"), None);
        assert_eq!(EntityMetaType::from_name("ZombieMetaMeta"), None);
        assert_eq!(EntityMetaType::from_name(""), None);
    }

    /// 注册表覆盖度：核心手写类型均能在注册表中找到。
    #[test]
    fn core_types_registered() {
        for name in [
            "ZombieMeta",
            "CreeperMeta",
            "SkeletonMeta",
            "PigMeta",
            "WolfMeta",
            "VillagerMeta",
            "IronGolemMeta",
            "SlimeMeta",
            "BatMeta",
            "FishingHookMeta",
            "PlayerMeta",
            "LivingEntityMeta",
            "ArmorStandMeta",
            "AbstractDisplayMeta",
            "BlockDisplayMeta",
            "ItemDisplayMeta",
            "TextDisplayMeta",
            "GhastMeta",
            "PhantomMeta",
            "SpiderMeta",
            "BlazeMeta",
            "EndermanMeta",
            "WardenMeta",
            "WitherMeta",
            "VexMeta",
            "PiglinMeta",
            "WitchMeta",
            "GuardianMeta",
            "AbstractArrowMeta",
            "ArrowMeta",
            "ThrownTridentMeta",
            "WitherSkullMeta",
            "BoatMeta",
            "FurnaceMinecartMeta",
            "CommandBlockMinecartMeta",
            "AbstractMinecartMeta",
            "AllayMeta",
            "ExperienceOrbMeta",
            "AreaEffectCloudMeta",
            "FoxMeta",
            "SnifferMeta",
            "CopperGolemMeta",
            "ShulkerMeta",
            "SnowGolemMeta",
        ] {
            assert!(
                EntityMetaType::from_name(name).is_some(),
                "核心类型 {name} 应已注册"
            );
        }
    }

    /// map ↔ 0x61/0x62 包条目互转：抽查新类型。
    #[test]
    fn new_types_map_roundtrip() {
        let player = PlayerMeta {
            additional_hearts: 2.5,
            score: 99,
            left_shoulder: Some(10),
            right_shoulder: Some(20),
        };
        assert_eq!(PlayerMeta::from_map(&player.to_map()), player);

        let armor_stand = ArmorStandMeta {
            is_small: true,
            has_arms: true,
            ..ArmorStandMeta::default()
        };
        assert_eq!(ArmorStandMeta::from_map(&armor_stand.to_map()), armor_stand);

        let abstract_display = AbstractDisplayMeta {
            interpolation_delay: 5,
            view_range: 8.0,
            ..AbstractDisplayMeta::default()
        };
        assert_eq!(
            AbstractDisplayMeta::from_map(&abstract_display.to_map()),
            abstract_display
        );

        let text_display = TextDisplayMeta {
            text: "hello".to_string(),
            line_width: 100,
            flags: 0x01,
            ..TextDisplayMeta::default()
        };
        assert_eq!(
            TextDisplayMeta::from_map(&text_display.to_map()),
            text_display
        );

        let ghast = GhastMeta { attacking: true };
        assert_eq!(GhastMeta::from_map(&ghast.to_map()), ghast);

        let phantom = PhantomMeta { size: 4 };
        assert_eq!(PhantomMeta::from_map(&phantom.to_map()), phantom);

        let spider = SpiderMeta { climbing: true };
        assert_eq!(SpiderMeta::from_map(&spider.to_map()), spider);

        let blaze = BlazeMeta { on_fire: true };
        assert_eq!(BlazeMeta::from_map(&blaze.to_map()), blaze);

        let enderman = EndermanMeta {
            carried_block: 7,
            screaming: true,
            staring: false,
        };
        assert_eq!(EndermanMeta::from_map(&enderman.to_map()), enderman);

        let warden = WardenMeta { anger_level: 4 };
        assert_eq!(WardenMeta::from_map(&warden.to_map()), warden);

        let wither = WitherMeta {
            center_head_target: 1,
            left_head_target: 2,
            right_head_target: 3,
            invulnerable_time: 100,
        };
        assert_eq!(WitherMeta::from_map(&wither.to_map()), wither);

        let vex = VexMeta { attacking: true };
        assert_eq!(VexMeta::from_map(&vex.to_map()), vex);

        let piglin = PiglinMeta {
            is_baby: true,
            charging_crossbow: true,
            dancing: false,
        };
        assert_eq!(PiglinMeta::from_map(&piglin.to_map()), piglin);

        let witch = WitchMeta {
            drinking_potion: true,
        };
        assert_eq!(WitchMeta::from_map(&witch.to_map()), witch);

        let guardian = GuardianMeta {
            retracting_spikes: true,
            target_entity_id: 42,
        };
        assert_eq!(GuardianMeta::from_map(&guardian.to_map()), guardian);

        let arrow = ArrowMeta { color: 0xFF0000 };
        assert_eq!(ArrowMeta::from_map(&arrow.to_map()), arrow);

        let trident = ThrownTridentMeta {
            loyalty_level: 3,
            has_enchantment_glint: true,
        };
        assert_eq!(ThrownTridentMeta::from_map(&trident.to_map()), trident);

        let wither_skull = WitherSkullMeta { invulnerable: true };
        assert_eq!(
            WitherSkullMeta::from_map(&wither_skull.to_map()),
            wither_skull
        );

        let boat = BoatMeta {
            left_paddle_turning: true,
            right_paddle_turning: false,
            splash_timer: 10,
        };
        assert_eq!(BoatMeta::from_map(&boat.to_map()), boat);

        let furnace_cart = FurnaceMinecartMeta { has_fuel: true };
        assert_eq!(
            FurnaceMinecartMeta::from_map(&furnace_cart.to_map()),
            furnace_cart
        );

        let command_cart = CommandBlockMinecartMeta {
            command: "say hello".to_string(),
            last_output: Some("Console: say hello".to_string()),
        };
        assert_eq!(
            CommandBlockMinecartMeta::from_map(&command_cart.to_map()),
            command_cart
        );

        let allay = AllayMeta {
            dancing: true,
            can_duplicate: false,
        };
        assert_eq!(AllayMeta::from_map(&allay.to_map()), allay);

        let experience_orb = ExperienceOrbMeta { value: 7 };
        assert_eq!(
            ExperienceOrbMeta::from_map(&experience_orb.to_map()),
            experience_orb
        );

        let area_effect_cloud = AreaEffectCloudMeta {
            radius: 3.5,
            waiting: true,
            particle_id: 42,
        };
        assert_eq!(
            AreaEffectCloudMeta::from_map(&area_effect_cloud.to_map()),
            area_effect_cloud
        );

        let fox = FoxMeta {
            sitting: true,
            crouching: true,
            interested: false,
            pouncing: true,
            sleeping: false,
            faceplanted: false,
            defending: true,
            first_uuid_low: 1,
            second_uuid_low: 2,
        };
        assert_eq!(FoxMeta::from_map(&fox.to_map()), fox);

        let sniffer = SnifferMeta {
            state: 3,
            drop_seed_at_tick: 100,
        };
        assert_eq!(SnifferMeta::from_map(&sniffer.to_map()), sniffer);

        let copper_golem = CopperGolemMeta {
            weather_state: 2,
            state: 1,
        };
        assert_eq!(
            CopperGolemMeta::from_map(&copper_golem.to_map()),
            copper_golem
        );

        let shulker = ShulkerMeta {
            attach_face: 2,
            shield_height: 4,
            color: 5,
        };
        assert_eq!(ShulkerMeta::from_map(&shulker.to_map()), shulker);

        let snow_golem = SnowGolemMeta { pumpkin_hat: true };
        assert_eq!(SnowGolemMeta::from_map(&snow_golem.to_map()), snow_golem);

        let abstract_arrow = AbstractArrowMeta {
            is_critical: true,
            is_no_clip: true,
            piercing_level: 4,
            is_in_ground: false,
        };
        assert_eq!(
            AbstractArrowMeta::from_map(&abstract_arrow.to_map()),
            abstract_arrow
        );

        let bogged = BoggedMeta { is_sheared: true };
        assert_eq!(BoggedMeta::from_map(&bogged.to_map()), bogged);

        let creaking = CreakingMeta {
            can_move: false,
            is_active: true,
            is_tearing_down: false,
            home_pos_x: 64,
        };
        assert_eq!(CreakingMeta::from_map(&creaking.to_map()), creaking);

        let zombie_villager = ZombieVillagerMeta {
            is_converting: true,
        };
        assert_eq!(
            ZombieVillagerMeta::from_map(&zombie_villager.to_map()),
            zombie_villager
        );

        let pillager = PillagerMeta {
            charging_crossbow: true,
        };
        assert_eq!(PillagerMeta::from_map(&pillager.to_map()), pillager);

        let spellcaster = SpellcasterIllagerMeta { spell: 3 };
        assert_eq!(
            SpellcasterIllagerMeta::from_map(&spellcaster.to_map()),
            spellcaster
        );

        let raider = RaiderMeta { celebrating: true };
        assert_eq!(RaiderMeta::from_map(&raider.to_map()), raider);

        let base_piglin = BasePiglinMeta {
            immune_to_zombification: true,
        };
        assert_eq!(BasePiglinMeta::from_map(&base_piglin.to_map()), base_piglin);
    }

    /// map ↔ 0x61/0x62 包条目互转：抽查新类型的 to_entries 长度。
    #[test]
    fn new_types_to_entries_count() {
        let player = PlayerMeta {
            additional_hearts: 0.0,
            score: 0,
            left_shoulder: None,
            right_shoulder: None,
        };
        let entries = player.to_map().to_entries();
        assert_eq!(entries.len(), 2); // index 0,1 only

        let armor_stand = ArmorStandMeta::default();
        let entries = armor_stand.to_map().to_entries();
        assert_eq!(entries.len(), 19); // index 0..=18

        let abstract_display = AbstractDisplayMeta::default();
        let entries = abstract_display.to_map().to_entries();
        assert_eq!(entries.len(), 10); // index 0..=9

        let text_display = TextDisplayMeta::default();
        let entries = text_display.to_map().to_entries();
        assert_eq!(entries.len(), 5); // index 0..=4

        let wither = WitherMeta {
            center_head_target: 0,
            left_head_target: 0,
            right_head_target: 0,
            invulnerable_time: 0,
        };
        assert_eq!(wither.to_map().to_entries().len(), 4);
    }

    /// from_name 全表：所有新增类型均可解析。
    #[test]
    fn all_new_types_parseable() {
        let names = [
            "PlayerMeta",
            "MannequinMeta",
            "ArmorStandMeta",
            "AbstractDisplayMeta",
            "BlockDisplayMeta",
            "ItemDisplayMeta",
            "TextDisplayMeta",
            "FlyingMeta",
            "GhastMeta",
            "PhantomMeta",
            "SnowGolemMeta",
            "ShulkerMeta",
            "CopperGolemMeta",
            "SpiderMeta",
            "BlazeMeta",
            "EndermanMeta",
            "WardenMeta",
            "WitherMeta",
            "MagmaCubeMeta",
            "VexMeta",
            "PiglinMeta",
            "WitchMeta",
            "EvokerMeta",
            "IllusionerMeta",
            "RavagerMeta",
            "VindicatorMeta",
            "AbstractIllagerMeta",
            "RaiderMeta",
            "SpellcasterIllagerMeta",
            "PillagerMeta",
            "CreakingMeta",
            "ZombieVillagerMeta",
            "HuskMeta",
            "DrownedMeta",
            "ZombifiedPiglinMeta",
            "PiglinBruteMeta",
            "ElderGuardianMeta",
            "GuardianMeta",
            "GiantMeta",
            "SilverfishMeta",
            "CaveSpiderMeta",
            "EndermiteMeta",
            "BreezeMeta",
            "StrayMeta",
            "WitherSkeletonMeta",
            "ParchedMeta",
            "BoggedMeta",
            "BasePiglinMeta",
            "AbstractArrowMeta",
            "ArrowMeta",
            "SpectralArrowMeta",
            "ThrownTridentMeta",
            "WitherSkullMeta",
            "DragonFireballMeta",
            "FireballMeta",
            "SmallFireballMeta",
            "SnowballMeta",
            "ThrownEggMeta",
            "ThrownEnderPearlMeta",
            "ThrownExperienceBottleMeta",
            "SplashPotionMeta",
            "LingeringPotionMeta",
            "EyeOfEnderMeta",
            "AbstractMinecartMeta",
            "MinecartMeta",
            "ChestMinecartMeta",
            "HopperMinecartMeta",
            "TntMinecartMeta",
            "SpawnerMinecartMeta",
            "FurnaceMinecartMeta",
            "CommandBlockMinecartMeta",
            "BoatMeta",
            "AllayMeta",
            "ExperienceOrbMeta",
            "AreaEffectCloudMeta",
            "FoxMeta",
            "SnifferMeta",
        ];
        for name in names {
            assert!(
                EntityMetaType::from_name(name).is_some(),
                "无法解析类型: {name}"
            );
        }
    }

    /// 新增实体 metadata 的 to_map / from_map roundtrip（≥10 种类型）。
    ///
    /// 覆盖：Player / ArmorStand / TextDisplay / Warden / Shulker / Arrow，
    /// 以及此前未测试的 Mannequin / BlockDisplay / ItemDisplay /
    /// SpectralArrow / ThrownExperienceBottle / AbstractArrow / ThrownTrident。
    #[test]
    fn new_entity_metadata_roundtrip() {
        // ── PlayerMeta ────────────────────────────────────────────────────
        let player = PlayerMeta {
            additional_hearts: 3.5,
            score: 128,
            left_shoulder: Some(5),
            right_shoulder: None,
        };
        assert_eq!(PlayerMeta::from_map(&player.to_map()), player);

        // ── ArmorStandMeta ────────────────────────────────────────────────
        let armor_stand = ArmorStandMeta {
            is_small: true,
            has_arms: true,
            has_no_base_plate: true,
            is_marker: false,
            head_rotation_x: 10,
            head_rotation_y: -30,
            head_rotation_z: 5,
            body_rotation_x: 0,
            body_rotation_y: 15,
            body_rotation_z: 0,
            left_arm_rotation_x: -30,
            left_arm_rotation_y: 10,
            left_arm_rotation_z: -10,
            right_arm_rotation_x: 30,
            right_arm_rotation_y: 10,
            right_arm_rotation_z: 10,
            left_leg_rotation_x: -15,
            left_leg_rotation_y: 0,
            left_leg_rotation_z: 5,
            right_leg_rotation_x: 15,
            right_leg_rotation_y: 0,
            right_leg_rotation_z: -5,
        };
        assert_eq!(ArmorStandMeta::from_map(&armor_stand.to_map()), armor_stand);

        // ── TextDisplayMeta ───────────────────────────────────────────────
        let text_display = TextDisplayMeta {
            text: "Hello §lwombat§r!".to_string(),
            line_width: 256,
            background_color: 0xFF884422u32 as i32,
            text_opacity: 127,
            flags: 0x03,
        };
        assert_eq!(
            TextDisplayMeta::from_map(&text_display.to_map()),
            text_display
        );

        // ── WardenMeta ────────────────────────────────────────────────────
        let warden = WardenMeta { anger_level: 3 };
        assert_eq!(WardenMeta::from_map(&warden.to_map()), warden);

        // ── ShulkerMeta ───────────────────────────────────────────────────
        let shulker = ShulkerMeta {
            attach_face: 1,
            shield_height: 2,
            color: 7,
        };
        assert_eq!(ShulkerMeta::from_map(&shulker.to_map()), shulker);

        // ── ArrowMeta ─────────────────────────────────────────────────────
        let arrow = ArrowMeta { color: 0x00FF00 };
        assert_eq!(ArrowMeta::from_map(&arrow.to_map()), arrow);

        // ── MannequinMeta（此前未覆盖）────────────────────────────────────
        let mannequin = MannequinMeta {
            profile: "test_profile".to_string(),
            immovable: true,
            description: Some("desc".to_string()),
        };
        assert_eq!(MannequinMeta::from_map(&mannequin.to_map()), mannequin);

        // ── BlockDisplayMeta（此前未覆盖）─────────────────────────────────
        let block_display = BlockDisplayMeta {
            displayed_block_state: 42,
        };
        assert_eq!(
            BlockDisplayMeta::from_map(&block_display.to_map()),
            block_display
        );

        // ── ItemDisplayMeta（此前未覆盖）──────────────────────────────────
        let item_display = ItemDisplayMeta {
            displayed_item: 17,
            display_context: 5,
        };
        assert_eq!(
            ItemDisplayMeta::from_map(&item_display.to_map()),
            item_display
        );

        // ── SpectralArrowMeta（此前未覆盖，空 struct）─────────────────────
        assert_eq!(
            SpectralArrowMeta::from_map(&SpectralArrowMeta.to_map()),
            SpectralArrowMeta
        );

        // ── ThrownExperienceBottleMeta（此前未覆盖）───────────────────────
        let exp_bottle = ThrownExperienceBottleMeta { value: 25 };
        assert_eq!(
            ThrownExperienceBottleMeta::from_map(&exp_bottle.to_map()),
            exp_bottle
        );

        // ── AbstractArrowMeta ─────────────────────────────────────────────
        let arrow_meta = AbstractArrowMeta {
            is_critical: true,
            is_no_clip: true,
            piercing_level: 2,
            is_in_ground: false,
        };
        assert_eq!(
            AbstractArrowMeta::from_map(&arrow_meta.to_map()),
            arrow_meta
        );

        // ── ThrownTridentMeta ─────────────────────────────────────────────
        let trident = ThrownTridentMeta {
            loyalty_level: 3,
            has_enchantment_glint: true,
        };
        assert_eq!(ThrownTridentMeta::from_map(&trident.to_map()), trident);
    }

    /// 新增实体 metadata：空 map 还原为默认值（注意：ArmorStandMeta 的旋转字段在
    /// 条目缺失时回 0，而非 struct default 的非零默认值，因此单独测试）。
    #[test]
    fn new_entity_metadata_defaults_from_empty_map() {
        assert_eq!(
            PlayerMeta::from_map(&EntityMetadataMap::new()),
            PlayerMeta::default()
        );
        assert_eq!(
            TextDisplayMeta::from_map(&EntityMetadataMap::new()),
            TextDisplayMeta::default()
        );
        assert_eq!(
            WardenMeta::from_map(&EntityMetadataMap::new()),
            WardenMeta::default()
        );
        assert_eq!(
            ShulkerMeta::from_map(&EntityMetadataMap::new()),
            ShulkerMeta::default()
        );
        assert_eq!(
            ArrowMeta::from_map(&EntityMetadataMap::new()),
            ArrowMeta::default()
        );
        // ArmorStandMeta：缺少旋转条目时各字段回 0（from_map 使用 unwrap_or(0)）。
        let from_empty = ArmorStandMeta::from_map(&EntityMetadataMap::new());
        assert_eq!(from_empty.is_small, false);
        assert_eq!(from_empty.has_arms, false);
        assert_eq!(from_empty.has_no_base_plate, false);
        assert_eq!(from_empty.is_marker, false);
        assert_eq!(from_empty.head_rotation_x, 0);
        assert_eq!(from_empty.left_arm_rotation_x, 0);
        assert_eq!(from_empty.right_leg_rotation_x, 0);
    }
}
