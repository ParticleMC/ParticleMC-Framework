// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 通用工具层：`MathUtils` / `TimeUtils` / `Validate`。
//!
//! 语义对齐框架的工具包，但**只补 Rust std
//! 尚未覆盖的能力**：`clamp` / `lerp` / 整数平方根、tick ↔ 毫秒 / 秒换算、
//! panic-free 的参数校验（返回 `Result`）。Java 中已由 std 等价覆盖的
//! （如 `min` / `max`、`abs`、`is_between` 等）不再重复实现。
//!
//! 变更标识符：`complete-missing-subsystems`（R15 utils/coordinate/thread 工具层）。
//! 见 `.specs/complete-missing-subsystems/spec.md`。

/// 数学工具命名空间（无状态，仅静态方法）。
pub struct MathUtils;

impl MathUtils {
    /// 把 `v` 钳制到 `[min, max]` 区间。
    ///
    /// `min > max` 时行为未定义（调用方应保证 `min <= max`）。
    pub fn clamp<T: PartialOrd>(v: T, min: T, max: T) -> T {
        if v < min {
            min
        } else if v > max {
            max
        } else {
            v
        }
    }

    /// 线性插值：`t == 0.0` 返回 `a`，`t == 1.0` 返回 `b`。
    pub fn lerp(a: f64, b: f64, t: f64) -> f64 {
        a + (b - a) * t
    }

    /// 整数平方根（向下取整），不涉及浮点运算。
    ///
    /// 委托 `u32::isqrt`（Rust 1.84 起标准库提供，本项目 MSRV 1.89 可用），
    /// 符合「Rust std 已覆盖的绝不重复实现」约定。
    pub fn isqrt(n: u32) -> u32 {
        n.isqrt()
    }
}

/// 时间换算工具：tick ↔ 毫秒 / 秒。
///
/// 对齐 Java `TickUtils` 语义：默认 Java 版客户端 20 TPS、每 tick 50ms。
pub struct TimeUtils;

impl TimeUtils {
    /// 每秒 tick 数（Java 版客户端）。
    pub const CLIENT_TPS: u64 = 20;
    /// 每 tick 毫秒数。
    pub const CLIENT_TICK_MS: u64 = 50;

    /// tick 数换算为毫秒（`ticks * 50`）。
    pub fn ticks_to_millis(ticks: u64) -> u64 {
        ticks * Self::CLIENT_TICK_MS
    }

    /// 毫秒数换算为 tick（`ms / 50`，向下取整）。
    pub fn millis_to_ticks(ms: u64) -> u64 {
        ms / Self::CLIENT_TICK_MS
    }

    /// tick 数换算为秒（`ticks / 20`，向下取整）。
    pub fn ticks_to_seconds(ticks: u64) -> u64 {
        ticks / Self::CLIENT_TPS
    }

    /// 分钟数换算为 tick（`minutes * 60 * 20`）。
    pub fn minutes_to_ticks(minutes: u64) -> u64 {
        minutes * 60 * Self::CLIENT_TPS
    }
}

/// panic-free 参数校验工具：校验失败返回 `Result::Err`，不触发 panic。
///
/// 对齐 Java `Check` 的语义，但以 `Result` 表达错误（符合项目章程
/// 「生产代码禁止 unwrap/expect，错误走显式分支」）。
pub struct Validate;

impl Validate {
    /// 校验字符串非空（`""` 视为非法）。
    pub fn not_empty(s: &str) -> Result<(), &'static str> {
        if s.is_empty() {
            Err("string must not be empty")
        } else {
            Ok(())
        }
    }

    /// 校验数值为正（`> 0`，零与负数视为非法）。
    pub fn positive(n: i64) -> Result<(), &'static str> {
        if n > 0 {
            Ok(())
        } else {
            Err("value must be positive")
        }
    }

    /// 校验 `v` 落在 `[min, max]` 闭区间内（含端点）。
    ///
    /// 若调用方传入 `min > max`，则任何值都无法通过（区间为空），恒返回 `Err`。
    pub fn in_range<T: PartialOrd + Copy>(v: T, min: T, max: T) -> Result<(), &'static str> {
        if v < min || v > max {
            Err("value out of range")
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn clamp_bounds_and_middle() {
        assert_eq!(MathUtils::clamp(5, 0, 10), 5);
        assert_eq!(MathUtils::clamp(-1, 0, 10), 0);
        assert_eq!(MathUtils::clamp(11, 0, 10), 10);
        // 浮点版本（显式标注类型以避免泛型 `clamp` 返回类型二义）。
        let mid: f64 = MathUtils::clamp(0.5, 0.0, 1.0);
        assert!((mid - 0.5).abs() < 1e-12);
        assert_eq!(MathUtils::clamp(-0.1, 0.0, 1.0), 0.0);
        assert_eq!(MathUtils::clamp(1.5, 0.0, 1.0), 1.0);
    }

    #[test]
    fn lerp_endpoints_and_midpoint() {
        assert_eq!(MathUtils::lerp(0.0, 10.0, 0.0), 0.0);
        assert_eq!(MathUtils::lerp(0.0, 10.0, 1.0), 10.0);
        assert_eq!(MathUtils::lerp(0.0, 10.0, 0.5), 5.0);
        assert_eq!(MathUtils::lerp(-4.0, 2.0, 0.25), -2.5);
    }

    #[test]
    fn isqrt_perfect_and_imperfect_squares() {
        // 完全平方数：期望值 = 各自平方根（字面量，避免浮点转换）。
        for (n, expected) in [(0u32, 0u32), (1, 1), (4, 2), (9, 3), (16, 4), (65536, 256)] {
            assert_eq!(MathUtils::isqrt(n), expected);
        }
        // 非完全平方：向下取整。
        assert_eq!(MathUtils::isqrt(2), 1);
        assert_eq!(MathUtils::isqrt(3), 1);
        assert_eq!(MathUtils::isqrt(15), 3);
        assert_eq!(MathUtils::isqrt(24), 4);
        // u32 边界。
        assert_eq!(MathUtils::isqrt(u32::MAX), 65535);
        // 逆性质：isqrt(n)² <= n < (isqrt(n)+1)²。
        for n in [7u32, 100, 999, 100_000, 4_000_000_000] {
            let r = MathUtils::isqrt(n);
            assert!(r * r <= n);
            assert!(n < (r + 1) * (r + 1));
        }
    }

    #[test]
    fn time_conversions() {
        assert_eq!(TimeUtils::ticks_to_millis(20), 1000);
        assert_eq!(TimeUtils::ticks_to_millis(1), 50);
        assert_eq!(TimeUtils::millis_to_ticks(1000), 20);
        assert_eq!(TimeUtils::millis_to_ticks(49), 0);
        assert_eq!(TimeUtils::millis_to_ticks(50), 1);
        assert_eq!(TimeUtils::ticks_to_seconds(20), 1);
        assert_eq!(TimeUtils::ticks_to_seconds(39), 1);
        assert_eq!(TimeUtils::ticks_to_seconds(40), 2);
        assert_eq!(TimeUtils::minutes_to_ticks(1), 1200);
        assert_eq!(TimeUtils::minutes_to_ticks(2), 2400);
        // roundtrip：100 tick ↔ 5000ms。
        assert_eq!(
            TimeUtils::ticks_to_millis(TimeUtils::millis_to_ticks(5000)),
            5000
        );
    }

    #[test]
    fn validate_not_empty() {
        assert_eq!(Validate::not_empty("abc"), Ok(()));
        assert_eq!(Validate::not_empty("a"), Ok(()));
        assert!(Validate::not_empty("").is_err());
    }

    #[test]
    fn validate_positive() {
        assert_eq!(Validate::positive(1), Ok(()));
        assert_eq!(Validate::positive(100), Ok(()));
        assert!(Validate::positive(0).is_err());
        assert!(Validate::positive(-5).is_err());
    }

    #[test]
    fn validate_in_range() {
        assert_eq!(Validate::in_range(5, 0, 10), Ok(()));
        assert_eq!(Validate::in_range(0, 0, 10), Ok(()));
        assert_eq!(Validate::in_range(10, 0, 10), Ok(()));
        assert!(Validate::in_range(-1, 0, 10).is_err());
        assert!(Validate::in_range(11, 0, 10).is_err());
        // min > max 时恒失败。
        assert!(Validate::in_range(5, 10, 0).is_err());
    }
}
