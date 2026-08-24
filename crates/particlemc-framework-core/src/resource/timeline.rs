// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 时间轴插值子系统（对齐 Java `net.minestom.server.world.timeline` 语义子集，T14）。
//!
//! Java `Timeline` 以 `period_ticks` + `Track(attribute, keyframes, ease)` 驱动
//! 世界属性随时间变化；本模块 v1 简化为一维（仅位置插值）：`Timeline` 持有按
//! tick 排序的 [`Keyframe`] 序列，`tick()` 推进当前 tick，
//! [`Timeline::current_position`] 在相邻关键帧间做线性插值（lerp）。
//!
//! 语义约定：
//!
//! - 当前 tick 早于首帧 → 恒等返回首帧位置（动画未开始，站桩）；
//! - 当前 tick 落在两帧之间 → 线性插值；
//! - 当前 tick **恰好等于**最后一帧 → 返回最后一帧位置；
//! - 当前 tick **越过**最后一帧（或关键帧为空）→ 已完成，返回 `None`。
//!
//! v1 仅插值位置；`Keyframe` 保留 yaw/pitch（旋转）字段供后续扩展，本版本不插值
//! 旋转。变更标识符：`complete-missing-subsystems`（R14）。

/// 时间轴插值属性（v1 仅支持 [`TimelineProperty::Position`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineProperty {
    /// 位置插值（`[x, y, z]`）。
    Position,
    /// 旋转插值（`yaw/pitch`，预留，v1 未实现）。
    Rotation,
}

/// 单个关键帧：在指定 tick 时点的位置与朝向。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Keyframe {
    /// 关键帧所在 tick。
    pub tick: u64,
    /// 世界坐标 `[x, y, z]`。
    pub position: [f64; 3],
    /// 偏航角（弧度，v1 仅承载不插值）。
    pub yaw: f32,
    /// 俯仰角（弧度，v1 仅承载不插值）。
    pub pitch: f32,
}

impl Keyframe {
    /// 以 tick 与位置构造（朝向为 0）。
    pub fn new(tick: u64, position: [f64; 3]) -> Self {
        Self {
            tick,
            position,
            yaw: 0.0,
            pitch: 0.0,
        }
    }

    /// 以 tick / 位置 / 朝向完整构造。
    pub fn with_rotation(tick: u64, position: [f64; 3], yaw: f32, pitch: f32) -> Self {
        Self {
            tick,
            position,
            yaw,
            pitch,
        }
    }
}

/// 关键帧时间轴（按 tick 升序，当前播放位置由 `current_tick` 推进）。
#[derive(Debug, Clone, PartialEq)]
pub struct Timeline {
    /// 关键帧序列（保持按 `tick` 升序）。
    pub keyframes: Vec<Keyframe>,
    /// 当前播放 tick。
    pub current_tick: u64,
}

impl Timeline {
    /// 构造空时间轴（`current_tick = 0`）。
    pub fn new() -> Self {
        Self {
            keyframes: Vec::new(),
            current_tick: 0,
        }
    }

    /// 追加关键帧并按 tick 升序重排（同 tick 多帧时后者在排序后位于末尾，
    /// 插值取最后命中的帧）。
    pub fn add_keyframe(&mut self, kf: Keyframe) {
        self.keyframes.push(kf);
        self.keyframes.sort_by_key(|k| k.tick);
    }

    /// 推进一个 tick（`current_tick += 1`，饱和防溢出）。
    pub fn tick(&mut self) {
        self.current_tick = self.current_tick.saturating_add(1);
    }

    /// 当前 tick 下的插值位置，见模块文档的语义约定。
    pub fn current_position(&self) -> Option<[f64; 3]> {
        if self.keyframes.is_empty() {
            return None;
        }
        // 最后一帧 tick <= 当前 tick 的索引（当前所在帧）。
        let lo = self
            .keyframes
            .iter()
            .rposition(|k| k.tick <= self.current_tick);
        let Some(lo) = lo else {
            // 当前 tick 早于首帧：站桩返回首帧位置。
            return self.keyframes.first().map(|k| k.position);
        };
        let frame = self.keyframes.get(lo)?;
        match self.keyframes.get(lo + 1) {
            None => {
                // 无后续帧：恰好停在最后一帧返回其位置；越过则已完成。
                if self.current_tick == frame.tick {
                    Some(frame.position)
                } else {
                    None
                }
            }
            Some(next) => {
                let span = next.tick.saturating_sub(frame.tick);
                // span 必 > 0（lo 为 tick <= current 的最后一帧，next.tick > current）。
                let t = (self.current_tick.saturating_sub(frame.tick)) as f64 / span as f64;
                Some(lerp(&frame.position, &next.position, t))
            }
        }
    }

    /// 时间轴是否播放完成：无关键帧，或当前 tick 已越过最后一帧。
    pub fn is_finished(&self) -> bool {
        match self.keyframes.last() {
            None => true,
            Some(last) => self.current_tick > last.tick,
        }
    }
}

impl Default for Timeline {
    fn default() -> Self {
        Self::new()
    }
}

/// 在 `a` / `b` 之间按比例 `t ∈ [0, 1]` 线性插值。
fn lerp(a: &[f64; 3], b: &[f64; 3], t: f64) -> [f64; 3] {
    let [ax, ay, az] = *a;
    let [bx, by, bz] = *b;
    [ax + (bx - ax) * t, ay + (by - ay) * t, az + (bz - az) * t]
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn assert_close(actual: [f64; 3], expected: [f64; 3]) {
        let eps = 1e-9;
        let [ax, ay, az] = actual;
        let [ex, ey, ez] = expected;
        assert!(
            (ax - ex).abs() < eps && (ay - ey).abs() < eps && (az - ez).abs() < eps,
            "位置不符：got {actual:?}, expected {expected:?}"
        );
    }

    #[test]
    fn add_keyframe_sorts_by_tick() {
        let mut tl = Timeline::new();
        tl.add_keyframe(Keyframe::new(10, [1.0, 0.0, 0.0]));
        tl.add_keyframe(Keyframe::new(0, [0.0, 0.0, 0.0]));
        tl.add_keyframe(Keyframe::new(5, [0.5, 0.0, 0.0]));
        let ticks: Vec<u64> = tl.keyframes.iter().map(|k| k.tick).collect();
        assert_eq!(ticks, vec![0, 5, 10]);
    }

    #[test]
    fn lerp_midpoint_is_halfway() {
        let mut tl = Timeline::new();
        tl.add_keyframe(Keyframe::new(0, [0.0, 0.0, 0.0]));
        tl.add_keyframe(Keyframe::new(10, [10.0, 20.0, 30.0]));
        tl.current_tick = 5; // 中点 → 线性插值一半
        let pos = tl.current_position().expect("区间内应有插值位置");
        assert_close(pos, [5.0, 10.0, 15.0]);
    }

    #[test]
    fn tick_progresses_and_advances_position() {
        let mut tl = Timeline::new();
        tl.add_keyframe(Keyframe::new(0, [0.0, 0.0, 0.0]));
        tl.add_keyframe(Keyframe::new(2, [2.0, 0.0, 0.0]));
        assert_eq!(tl.current_tick, 0);
        let at_start = tl.current_position().expect("tick0 应在首帧");
        assert_close(at_start, [0.0, 0.0, 0.0]);
        tl.tick();
        assert_eq!(tl.current_tick, 1);
        let at_half = tl.current_position().expect("tick1 应插值");
        assert_close(at_half, [1.0, 0.0, 0.0]);
        tl.tick();
        assert_eq!(tl.current_tick, 2);
        let at_end = tl.current_position().expect("tick2 应落在末帧");
        assert_close(at_end, [2.0, 0.0, 0.0]);
    }

    #[test]
    fn finished_returns_none() {
        let mut tl = Timeline::new();
        tl.add_keyframe(Keyframe::new(0, [0.0, 0.0, 0.0]));
        tl.add_keyframe(Keyframe::new(10, [1.0, 0.0, 0.0]));
        tl.current_tick = 10; // 恰好等于末帧：返回末帧，未完成。
        assert!(!tl.is_finished());
        assert!(tl.current_position().is_some());
        tl.tick(); // 越过末帧：完成，None。
        assert!(tl.is_finished());
        assert_eq!(tl.current_position(), None);
        // 空时间轴直接完成。
        assert!(Timeline::new().is_finished());
        assert_eq!(Timeline::new().current_position(), None);
    }

    #[test]
    fn single_keyframe_is_identity() {
        let mut tl = Timeline::new();
        tl.add_keyframe(Keyframe::new(3, [7.0, 8.0, 9.0]));
        // 早于首帧：站桩。
        tl.current_tick = 0;
        assert_close(
            tl.current_position().expect("早于首帧站桩"),
            [7.0, 8.0, 9.0],
        );
        // 恰好末帧。
        tl.current_tick = 3;
        assert_close(tl.current_position().expect("末帧恒等"), [7.0, 8.0, 9.0]);
        // 越过 → 完成。
        tl.current_tick = 4;
        assert_eq!(tl.current_position(), None);
        assert!(tl.is_finished());
    }
}
