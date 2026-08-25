// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 固定步长调度：ParticleMC-Framework 的 20Hz tick（20 TPS）。
//!
//! 借助自研 `Schedule` 内嵌的 `FixedClock` 实现固定步长循环（替代旧 ECS 方案
//! `Schedule` + `Time<Fixed>`，RM1）。`Schedule::new` 默认即为 20Hz，这里
//! 显式确认，对应 旧 ECS 方案 `Time::<Fixed>::from_hz(20.0)` 的语义。

use crate::app::App;

/// ParticleMC-Framework 目标 tick 频率（赫兹），即每秒 20 个逻辑 tick。
pub const TICK_RATE_HZ: f64 = 20.0;

/// 将 App 的固定时间步长配置为 20Hz。
///
/// `Schedule` 内嵌 `FixedClock`（`Schedule::new` 默认 20Hz），此处显式覆写为
/// `TICK_RATE_HZ`，对应 旧 ECS 方案 `TimePlugin` + `Time::<Fixed>::from_hz(20.0)` 的
/// 装配语义（旧 ECS 方案 `TimePlugin` 已不需要）。
pub fn configure_20hz(app: &mut App) {
    app.schedule.set_fixed_hz(TICK_RATE_HZ);
}
