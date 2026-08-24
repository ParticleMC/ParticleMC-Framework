// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! tick 管线：推进调度器时钟并执行到期任务。
//!
//! [`TickCounter`] 每 tick 递增一次，作为 [`TaskScheduler`] 的调度时钟源；
//! [`scheduler_tick`] 在 `tick_begin` 之后运行，执行全部到期任务
//! （延迟 / 周期 / 定点，可取消）。见 `.specs/implement-framework-capabilities/`。

use crate::prelude::ResMut;

use crate::resource::scheduler_manager::TaskScheduler;

/// tick 计数器（`scheduler_tick` 每 tick 递增，作为调度时钟源）。
#[derive(Default, Debug, Clone, Copy)]
pub struct TickCounter(pub u64);

/// 推进时钟并执行到期任务。
pub fn scheduler_tick(mut counter: ResMut<TickCounter>, mut scheduler: ResMut<TaskScheduler>) {
    counter.0 = counter.0.saturating_add(1);
    scheduler.tick(counter.0);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use crate::app::App;

    use super::*;

    #[test]
    fn tick_counter_increments_and_scheduler_runs() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let mut app = App::new();
        app.init_resource::<TickCounter>();
        app.init_resource::<TaskScheduler>();
        app.add_systems(scheduler_tick);

        // 用原子标记观察闭包执行（闭包 move 捕获 Copy 值无意义）。
        let done = Arc::new(AtomicBool::new(false));
        let done_clone = Arc::clone(&done);
        app.world_mut()
            .resource_mut::<TaskScheduler>()
            .unwrap()
            .schedule_after(3, move || {
                done_clone.store(true, Ordering::SeqCst);
            });
        // 前 3 tick 不执行，第 4 tick 执行。
        for _ in 0..4 {
            app.update();
        }
        assert!(done.load(Ordering::SeqCst), "第 4 tick 后延迟任务应已执行");
        assert_eq!(app.world().resource::<TickCounter>().unwrap().0, 4);
    }
}
