//! 定时器便捷层（对齐 Java `net.minestom.server.timer` 语义，T14）。
//!
//! Java Minestom 的 `Scheduler` / `Schedulable` 提供 `scheduleNextTick` /
//! `scheduleAfter` / `scheduleRepeating` 等便捷调度；本仓库任务调度底座为
//! [`crate::resource::scheduler_manager::TaskScheduler`]（R9，`schedule_after` /
//! `schedule_repeat` / `schedule_at_tick` / `cancel` + `TaskId`）。
//!
//! 本模块提供两层便捷 API（均委托 `TaskScheduler`，语义一一对应）：
//!
//! - [`Timer`]：无状态命名空间，`after` / `repeat` / `at_tick` 静态方法；
//! - [`Schedulable`]：trait，为 `TaskScheduler` 实现，便于泛型约束与测试桩。
//!
//! 与 Java `TaskSchedule.duration/tick` 对齐：时长以 **tick** 计（1.21.11
//! 服务器 20 TPS），无毫秒换算。变更标识符：`complete-missing-subsystems`（R14）。

use crate::resource::scheduler_manager::{TaskId, TaskScheduler};

/// 定时器命名空间（无字段，仅提供便捷静态方法）。
///
/// 全部方法委托 [`TaskScheduler`]：`after` 对应 `schedule_after`、
/// `repeat` 对应 `schedule_repeat`、`at_tick` 对应 `schedule_at_tick`。
pub struct Timer;

impl Timer {
    /// 延迟 `ticks` 后执行一次任务。
    ///
    /// 到期点为调度时 `TaskScheduler` 记录的最新 tick + `ticks`。
    pub fn after(
        task_scheduler: &mut TaskScheduler,
        ticks: u64,
        f: impl FnOnce() + Send + Sync + 'static,
    ) -> TaskId {
        task_scheduler.schedule_after(ticks, f)
    }

    /// 每 `ticks` 执行一次周期任务，返回可取消的 [`TaskId`]。
    pub fn repeat(
        task_scheduler: &mut TaskScheduler,
        ticks: u64,
        f: impl FnMut() + Send + Sync + 'static,
    ) -> TaskId {
        task_scheduler.schedule_repeat(ticks, f)
    }

    /// 在绝对 tick 执行一次任务。
    pub fn at_tick(
        task_scheduler: &mut TaskScheduler,
        ticks: u64,
        f: impl FnOnce() + Send + Sync + 'static,
    ) -> TaskId {
        task_scheduler.schedule_at_tick(ticks, f)
    }
}

/// 可调度 trait：为 [`TaskScheduler`] 实现，语义与其固有方法一一对应。
///
/// 与固有方法同名（委托实现），供需要 trait 抽象的调用方使用
/// （`Schedulable::schedule_after(&mut sched, ...)`）。
pub trait Schedulable {
    /// 延迟 `ticks` 后执行一次。
    fn schedule_after(&mut self, ticks: u64, f: impl FnOnce() + Send + Sync + 'static) -> TaskId;

    /// 每 `ticks` 执行一次周期任务。
    fn schedule_repeat(&mut self, ticks: u64, f: impl FnMut() + Send + Sync + 'static) -> TaskId;

    /// 在绝对 tick 执行一次。
    fn schedule_at_tick(&mut self, ticks: u64, f: impl FnOnce() + Send + Sync + 'static) -> TaskId;
}

impl Schedulable for TaskScheduler {
    fn schedule_after(&mut self, ticks: u64, f: impl FnOnce() + Send + Sync + 'static) -> TaskId {
        TaskScheduler::schedule_after(self, ticks, f)
    }

    fn schedule_repeat(&mut self, ticks: u64, f: impl FnMut() + Send + Sync + 'static) -> TaskId {
        TaskScheduler::schedule_repeat(self, ticks, f)
    }

    fn schedule_at_tick(&mut self, ticks: u64, f: impl FnOnce() + Send + Sync + 'static) -> TaskId {
        TaskScheduler::schedule_at_tick(self, ticks, f)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn after_executes_once_when_tick_reaches_due() {
        let mut sched = TaskScheduler::default();
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let id = Timer::after(&mut sched, 3, move || {
            c.fetch_add(1, Ordering::Relaxed);
        });
        // 未到期不触发。
        for t in 1..=2 {
            sched.tick(t);
            assert_eq!(counter.load(Ordering::Relaxed), 0);
        }
        // 到期 tick=3 触发一次，之后不再触发。
        sched.tick(3);
        assert_eq!(counter.load(Ordering::Relaxed), 1);
        sched.tick(4);
        assert_eq!(counter.load(Ordering::Relaxed), 1);
        assert_eq!(sched.pending(), 0);
        // 已执行任务不可再取消。
        assert!(!sched.cancel(id));
    }

    #[test]
    fn repeat_fires_every_n_ticks_and_cancelable() {
        let mut sched = TaskScheduler::default();
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let id = Timer::repeat(&mut sched, 2, move || {
            c.fetch_add(1, Ordering::Relaxed);
        });
        // 到期 tick：2、4、6。
        sched.tick(2);
        assert_eq!(counter.load(Ordering::Relaxed), 1);
        sched.tick(4);
        assert_eq!(counter.load(Ordering::Relaxed), 2);
        assert!(sched.cancel(id));
        sched.tick(6);
        assert_eq!(counter.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn schedulable_trait_delegates_to_scheduler() {
        let mut sched = TaskScheduler::default();
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        Schedulable::schedule_at_tick(&mut sched, 5, move || {
            c.fetch_add(1, Ordering::Relaxed);
        });
        sched.tick(4);
        assert_eq!(counter.load(Ordering::Relaxed), 0);
        sched.tick(5);
        assert_eq!(counter.load(Ordering::Relaxed), 1);
    }
}
