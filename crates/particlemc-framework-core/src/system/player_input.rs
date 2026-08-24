//! tick 管线第二步：把 `NetworkEvent::PlayerMoveRaw` 应用到玩家实体坐标，并派生 `PlayerMove` 事件。
//!
//! 由 `network_receive` 从 `PlayerPosition` / `PlayerPositionAndRotation` 派生
//! 的移动上报在此落盘到对应玩家的 `Position` 组件（含朝向）；坐标实际变化时
//! 额外写入 `PlayerMove` 事件，供后续系统（如移动同步）消费。

use crate::prelude::{MessageReader, MessageWriter, Res};

use crate::component::Position;
use crate::event::{NetworkEvent, PlayerMove};
use crate::resource::connection_manager::ConnectionManager;
use particlemc_framework_ecs::scheduler::{InstanceScheduler, WorldId};

/// 从网络移动上报更新玩家坐标，并在坐标变化时派生 `PlayerMove` 事件。
///
/// 玩家实体已迁入实例 World（R11.2）：本系统留主 World，经连接运行时记录的
/// `world_id` 跨 World 定位该玩家所在实例，写其 `Position` 组件。
pub fn player_input(
    scheduler: Res<InstanceScheduler>,
    connections: Res<ConnectionManager>,
    events: MessageReader<NetworkEvent>,
    mut moves: MessageWriter<PlayerMove>,
) {
    for event in events.read() {
        if let NetworkEvent::PlayerMoveRaw {
            conn_id,
            position,
            yaw,
            pitch,
            ..
        } = event
            && let Some(entity) = connections.entity_of(*conn_id)
        {
            let wid = connections
                .get(*conn_id)
                .map(|rt| rt.world_id)
                .unwrap_or(WorldId(0));
            if let Some(mut guard) = scheduler.lock_world(wid)
                && let Some(pos) = guard.world().get_mut::<Position>(entity)
            {
                let from = *pos;
                pos.x = position.x;
                pos.y = position.y;
                pos.z = position.z;
                pos.yaw = *yaw;
                pos.pitch = *pitch;
                // 仅坐标实际变化时派发事件，避免每 tick 产生空移动。
                if from != *pos {
                    moves.write(PlayerMove {
                        entity,
                        from,
                        to: *pos,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    use crate::app::App;
    use crate::component::{InstanceRef, Player, Position};
    use crate::event::PlayerMove;
    use crate::plugin::McServerPlugin;
    use crate::prelude::ResMut;
    use crate::test_support::{
        current_test_instance, ensure_test_instance, read_instance, spawn_into_instance,
    };
    use uuid::Uuid;

    /// 收集测试期间产生的 `PlayerMove` 事件。
    #[derive(Default)]
    struct Moved(Vec<PlayerMove>);

    fn collect_moves(reader: MessageReader<PlayerMove>, mut moved: ResMut<Moved>) {
        moved.0.extend(reader.read().cloned());
    }

    fn app_with_player() -> (App, crate::prelude::Entity) {
        let mut app = App::new();
        app.add_plugins(McServerPlugin::new());

        // 手动推进时间：连续 app.update() 无真实时间流逝，Schedule 不会执行
        //（Time<Fixed> 累加不足）。ManualDuration 使每次 update 推进 50ms（一个 20Hz 步长）。
        app.world_mut()
            .insert_resource(crate::app::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(50),
            ));

        // 建立 conn → entity 映射并生成玩家（落入实例 World，R11.2）。
        let inst = ensure_test_instance(&mut app);
        let entity = spawn_into_instance(
            &mut app,
            inst,
            (
                Player::new(Uuid::nil(), "tester"),
                Position::new(0.0, 64.0, 0.0),
                InstanceRef(inst),
            ),
        );
        app.world_mut()
            .resource_mut::<ConnectionManager>()
            .unwrap()
            .open(1, None)
            .entity = Some(entity);

        app.world_mut().init_resource::<Moved>();
        app.add_systems(collect_moves);
        // collect_moves 读取 player_input 同 tick 内写入的 PlayerMove，须在其后运行。
        // 自建调度器对无依赖系统（collect_moves）按 Kahn 分层会排到依赖系统
        // （player_input 依赖 network_receive）之前，故显式声明顺序。
        app.after(collect_moves, player_input);

        (app, entity)
    }

    #[test]
    fn player_move_raw_updates_position() {
        let (mut app, entity) = app_with_player();

        // 写入移动上报
        app.world_mut().write(NetworkEvent::PlayerMoveRaw {
            conn_id: 1,
            position: Position::new(10.0, 64.0, 20.0),
            yaw: 0.5,
            pitch: -0.5,
            grounded: true,
        });

        // 自研 ECS 消息时序：write_message 后需跨一帧——第一次 update 让
        // FixedPostUpdate 的 signal_message_update_system 置 Ready，第二次 update
        // 的 First 才把消息推给 MessageReader，player_input 才能读到。
        app.update();
        app.update();

        let inst = current_test_instance(&app);
        let pos = read_instance::<Position>(&mut app, inst, entity).expect("玩家应挂载 Position");
        assert_eq!(pos.x, 10.0);
        assert_eq!(pos.z, 20.0);
        assert_eq!(pos.yaw, 0.5);
        assert_eq!(pos.pitch, -0.5);

        // player_input 在坐标变化后派生 PlayerMove：from 为旧坐标，to 为新坐标
        let moved = app.world().resource::<Moved>().unwrap().0.clone();
        let mv = moved.first().unwrap();
        assert_eq!(mv.entity, entity);
        assert_eq!(mv.from, Position::new(0.0, 64.0, 0.0));
        assert_eq!(mv.to, Position::with_rotation(10.0, 64.0, 20.0, 0.5, -0.5));
    }

    #[test]
    fn player_move_raw_same_position_emits_no_event() {
        let (mut app, _entity) = app_with_player();

        // 上报与当前位置完全相同（仅朝向也一致），坐标未变化时不应派生 PlayerMove
        app.world_mut().write(NetworkEvent::PlayerMoveRaw {
            conn_id: 1,
            position: Position::new(0.0, 64.0, 0.0),
            yaw: 0.0,
            pitch: 0.0,
            grounded: true,
        });

        app.update();
        app.update();

        assert!(app.world().resource::<Moved>().unwrap().0.is_empty());
    }
}
