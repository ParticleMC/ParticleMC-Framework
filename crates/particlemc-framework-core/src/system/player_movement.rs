//! tick 管线第三步：玩家位移整合（服务端驱动部分）。
//!
//! 客户端上报的坐标已在 `player_input` 中权威落盘。本阶段对服务端驱动的位移
//! （击退、速度效果等）按 `Velocity` 对 `Position` 做积分：
//! `delta = velocity * dt`（dt 由 `scheduler_tick` 更新到 `Res<TickDelta>`）。
//! 积分后清零速度，以便下一个 tick 重新应用新的速度效果。

use crate::prelude::{Query, Res, With};

use crate::component::{Player, Position, Velocity};

/// 依据速度对玩家坐标做服务端积分，并清零速度。
pub fn player_movement(
    mut query: Query<(&mut Position, &mut Velocity), With<Player>>,
    tick_delta: Res<f64>,
) {
    let dt = *tick_delta;
    for (pos, vel) in query.iter_mut() {
        // 按速度积分位移（方块单位）。
        pos.x += vel.x * dt;
        pos.y += vel.y * dt;
        pos.z += vel.z * dt;
        // 速度清零（下一 tick 由其他系统重新施加）。
        vel.x = 0.0;
        vel.y = 0.0;
        vel.z = 0.0;
    }
}
