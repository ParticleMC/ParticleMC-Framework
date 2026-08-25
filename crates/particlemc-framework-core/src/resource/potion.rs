// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 药水子系统（对齐 Java `net.minestom.server.potion` 语义，T14）。
//!
//! 对应框架的 `PotionEffect` / `TimedPotion` / `PotionEffects` / `PotionType`
//! 概念：`PotionEffect` 描述单一药水效果（名称 / 注册表 id / 时长 / 等级），
//! `TimedPotion` 记录该效果的过期 tick，`PotionEffects` 挂实体（旧 ECS 方案 `Component`）
//! 统一管理一实体的全部效果（同 id 覆盖、tick 过期移除）。
//!
//! `PotionType` 枚举序位对齐 Minecraft 1.21.11 数据包 `potion_type` 注册表
//! （即 Java `PotionTypeImpl` 从 `RegistryData` 加载的序位，见
//! `resources/data/generic/potion_type.toml`，Water=0、Swiftness=13、Strength=34、
//! Regeneration=31 等），`PotionEffect.id` 对齐 `potion_effect` 注册表序位
//! （见 `resources/data/potion_effects.toml`，speed=0、strength=4、regeneration=9），
//! 由调用方经 [`PotionType::effect_id`] 或 [`PotionEffect::with_id`] 补齐。
//!
//! 变更标识符：`complete-missing-subsystems`（R14）。

use crate::prelude::Component;

/// 单一药水效果（值类型）。
///
/// `id` 对应 `potion_effect` 注册表协议序号（如 `minecraft:strength` = 4）；
/// [`PotionEffect::new`] 不接收 id（由注册表补齐），需通过
/// [`PotionEffect::with_id`] 或 [`PotionType::effect_id`] 设置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PotionEffect {
    /// 效果名（如 `minecraft:strength`）。
    pub name: String,
    /// `potion_effect` 注册表 id（同 id 视为同一效果，用于覆盖判定）。
    pub id: u32,
    /// 效果时长（tick；负数视为无限）。
    pub duration: i32,
    /// 效果等级（0 为一级）。
    pub amplifier: i32,
}

impl PotionEffect {
    /// 以名称 / 时长 / 等级构造效果（id 初始为 0，由注册表补）。
    pub fn new(name: &str, duration: i32, amplifier: i32) -> Self {
        Self {
            name: name.to_owned(),
            id: 0,
            duration,
            amplifier,
        }
    }

    /// 设置 `potion_effect` 注册表 id，返回自身（builder 风格）。
    pub fn with_id(mut self, id: u32) -> Self {
        self.id = id;
        self
    }
}

/// 带过期 tick 的药水效果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimedPotion {
    /// 效果本体。
    pub effect: PotionEffect,
    /// 过期 tick（`tick` 到该 tick 之后移除，即严格大于当前 tick 才算存活）。
    pub expires_at_tick: u64,
}

impl TimedPotion {
    /// 以效果与过期 tick 构造。
    pub fn new(effect: PotionEffect, expires_at_tick: u64) -> Self {
        Self {
            effect,
            expires_at_tick,
        }
    }

    /// 当前剩余 tick 数（`expires_at_tick - current_tick`，下溢钳 0）。
    pub fn remaining_ticks(&self, current_tick: u64) -> u64 {
        self.expires_at_tick.saturating_sub(current_tick)
    }
}

/// 实体持有的药水效果集合（旧 ECS 方案 `Component`）。
#[derive(Component, Debug, Clone, Default, PartialEq)]
#[component(storage = "sparse")]
pub struct PotionEffects {
    /// 效果列表（顺序不定，查询经 [`has`](Self::has) / [`active_effects`](Self::active_effects)）。
    pub effects: Vec<TimedPotion>,
}

impl PotionEffects {
    /// 施加一个效果：同 id（`effect.id`）已存在时**覆盖**其效果并重置过期点；
    /// 不存在则追加。
    ///
    /// 过期点 = `current_tick + effect.duration`；负时长（无限）以 `u64::MAX` 表示。
    pub fn apply(&mut self, effect: PotionEffect, current_tick: u64) {
        // 负时长视为无限效果（对齐 Java `Potion` 的无限语义简化版）。
        let duration_ticks: u64 = u64::try_from(effect.duration).unwrap_or(u64::MAX);
        let expires_at_tick = current_tick.saturating_add(duration_ticks);
        if let Some(existing) = self.effects.iter_mut().find(|tp| tp.effect.id == effect.id) {
            existing.effect = effect;
            existing.expires_at_tick = expires_at_tick;
        } else {
            self.effects.push(TimedPotion::new(effect, expires_at_tick));
        }
    }

    /// 推进一个 tick：移除全部 `expires_at_tick <= current_tick` 的过期效果。
    pub fn tick(&mut self, current_tick: u64) {
        self.effects.retain(|tp| tp.expires_at_tick > current_tick);
    }

    /// 是否持有指定 id 的效果（含已到期未推进的）。
    pub fn has(&self, effect_id: u32) -> bool {
        self.effects.iter().any(|tp| tp.effect.id == effect_id)
    }

    /// 迭代全部（含）效果。
    pub fn active_effects(&self) -> impl Iterator<Item = &TimedPotion> {
        self.effects.iter()
    }
}

/// 药水类型（`potion_type` 注册表枚举子集）。
///
/// 序位对齐 Minecraft 1.21.11 数据包 `potion_type` 注册表（Java
/// `PotionTypeImpl` 经 `RegistryData` 加载的 id，见
/// `resources/data/generic/potion_type.toml`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PotionType {
    /// `minecraft:water`（id 0）。
    Water,
    /// `minecraft:night_vision`（id 4）。
    NightVision,
    /// `minecraft:invisibility`（id 6）。
    Invisibility,
    /// `minecraft:fire_resistance`（id 11）。
    FireResistance,
    /// `minecraft:swiftness`（id 13）。
    Swiftness,
    /// `minecraft:slowness`（id 16）。
    Slowness,
    /// `minecraft:water_breathing`（id 22）。
    WaterBreathing,
    /// `minecraft:healing`（id 24）。
    Healing,
    /// `minecraft:poison`（id 28）。
    Poison,
    /// `minecraft:regeneration`（id 31）。
    Regeneration,
    /// `minecraft:strength`（id 34）。
    Strength,
}

impl PotionType {
    /// `potion_type` 注册表 id。
    pub fn id(self) -> u32 {
        match self {
            PotionType::Water => 0,
            PotionType::NightVision => 4,
            PotionType::Invisibility => 6,
            PotionType::FireResistance => 11,
            PotionType::Swiftness => 13,
            PotionType::Slowness => 16,
            PotionType::WaterBreathing => 22,
            PotionType::Healing => 24,
            PotionType::Poison => 28,
            PotionType::Regeneration => 31,
            PotionType::Strength => 34,
        }
    }

    /// `potion_type` 注册表命名空间名称。
    pub fn name(self) -> &'static str {
        match self {
            PotionType::Water => "minecraft:water",
            PotionType::NightVision => "minecraft:night_vision",
            PotionType::Invisibility => "minecraft:invisibility",
            PotionType::FireResistance => "minecraft:fire_resistance",
            PotionType::Swiftness => "minecraft:swiftness",
            PotionType::Slowness => "minecraft:slowness",
            PotionType::WaterBreathing => "minecraft:water_breathing",
            PotionType::Healing => "minecraft:healing",
            PotionType::Poison => "minecraft:poison",
            PotionType::Regeneration => "minecraft:regeneration",
            PotionType::Strength => "minecraft:strength",
        }
    }

    /// 该药水类型对应的 `potion_effect` 注册表 id
    /// （对齐 `resources/data/potion_effects.toml`，如 Swiftness→speed=0、
    /// Strength→strength=4、Regeneration→regeneration=9、Poison→poison=18）。
    pub fn effect_id(self) -> u32 {
        match self {
            PotionType::Water => 0,
            PotionType::NightVision => 15,
            PotionType::Invisibility => 13,
            PotionType::FireResistance => 11,
            PotionType::Swiftness => 0,
            PotionType::Slowness => 1,
            PotionType::WaterBreathing => 12,
            PotionType::Healing => 5,
            PotionType::Poison => 18,
            PotionType::Regeneration => 9,
            PotionType::Strength => 4,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn strength_effect(duration: i32) -> PotionEffect {
        PotionEffect::new("minecraft:strength", duration, 1).with_id(4)
    }

    #[test]
    fn apply_adds_timed_potion_and_has_detects() {
        let mut effects = PotionEffects::default();
        assert!(!effects.has(4));
        effects.apply(strength_effect(100), 0);
        assert!(effects.has(4));
        let all: Vec<&TimedPotion> = effects.active_effects().collect();
        assert_eq!(all.len(), 1);
        let timed = all.first().expect("施加后应有一条效果");
        assert_eq!(timed.expires_at_tick, 100);
        // remaining_ticks 递减。
        assert_eq!(timed.remaining_ticks(0), 100);
        assert_eq!(timed.remaining_ticks(40), 60);
        assert_eq!(timed.remaining_ticks(200), 0); // 下溢钳 0
    }

    #[test]
    fn apply_same_id_overwrites_in_place() {
        let mut effects = PotionEffects::default();
        effects.apply(strength_effect(100), 0);
        effects.apply(
            PotionEffect::new("minecraft:strength", 50, 3).with_id(4),
            10,
        );
        // 同 id（4）覆盖：列表不增长，时长 / 等级已更新。
        assert_eq!(effects.effects.len(), 1);
        let overwritten = effects.effects.first().expect("覆盖后应保留一条效果");
        assert_eq!(overwritten.effect.duration, 50);
        assert_eq!(overwritten.effect.amplifier, 3);
        assert_eq!(overwritten.expires_at_tick, 60); // 10 + 50
    }

    #[test]
    fn tick_removes_expired_effects() {
        let mut effects = PotionEffects::default();
        effects.apply(strength_effect(100), 0); // 过期点 100
        effects.apply(PotionEffect::new("minecraft:poison", 30, 0).with_id(18), 0); // 过期点 30
        effects.tick(30); // 30 时 poison 已到期（严格大于才算存活）
        assert!(!effects.has(18));
        assert!(effects.has(4));
        effects.tick(100);
        assert!(!effects.has(4));
        assert!(effects.effects.is_empty());
    }

    #[test]
    fn negative_duration_means_infinite() {
        let mut effects = PotionEffects::default();
        effects.apply(strength_effect(-1), 0);
        let infinite = effects.effects.first().expect("应有一条无限效果");
        assert_eq!(infinite.expires_at_tick, u64::MAX);
        effects.tick(u64::MAX - 1);
        assert!(effects.has(4));
    }

    #[test]
    fn potion_type_matches_registry_ids() {
        // 序位对齐 potion_type.toml（Minecraft 1.21.11）。
        assert_eq!(PotionType::Water.id(), 0);
        assert_eq!(PotionType::Swiftness.id(), 13);
        assert_eq!(PotionType::Regeneration.id(), 31);
        assert_eq!(PotionType::Strength.id(), 34);
        assert_eq!(PotionType::Strength.name(), "minecraft:strength");
        // effect_id 对齐 potion_effects.toml。
        assert_eq!(PotionType::Swiftness.effect_id(), 0); // speed
        assert_eq!(PotionType::Strength.effect_id(), 4);
    }
}
