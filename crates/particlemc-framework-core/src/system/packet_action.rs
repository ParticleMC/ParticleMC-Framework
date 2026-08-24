//! tick 管线：消费动作类 serverbound 包并经事件总线派发。
//!
//! `network_receive` 把框架关注的动作包（交互 / 动作 / 动画 / 使用物品 /
//! 放置）原样写入 `ClientNetworks.packet_inbox`，本系统在本 tick 稍后
//! （`network_receive` 之后、`network_send` 之前）取出并反查玩家实体，
//! 经 [`EventBus`] 派发对应框架动作事件。
//! 解耦设计避免 `network_receive` 增参（`SystemParam` 16 上限）。
//! 见 `.specs/implement-framework-capabilities/`。

use crate::prelude::{MessageWriter, Res, ResMut};

use crate::component::Position;
use crate::event::bus::EventBus;
use crate::event::inventory::inventory_click::ClickAction;
use crate::event::inventory::{CreativeInventoryActionEvent, WindowButtonClickEvent};
use crate::event::player::{BookEditEvent, PluginMessageEvent};
use crate::event::{BlockPlace, EntityInteract, PlayerActionEvent, PlayerAnimation, PlayerUseItem};
use crate::network::client::ClientNetworks;
use crate::protocol::packets::InboundPacket;
use crate::resource::connection_manager::ConnectionManager;

/// 消费收件箱：反查玩家实体并将框架事件写入消息收件箱，供
/// [`super::block_interaction_validator::block_interaction_validator`]
/// 消费并派发至 [`EventBus`]。无绑定实体的连接被跳过（不 panic）。
///
/// `PlayerActionEvent` 与 `BlockPlace` 事件写入消息收件箱；
/// 其余事件仍按原有方式直接派发至 [`EventBus`]。
pub fn packet_action_system(
    mut clients: ResMut<ClientNetworks>,
    connections: Res<ConnectionManager>,
    mut event_bus: ResMut<EventBus>,
    mut player_action_writer: MessageWriter<PlayerActionEvent>,
    mut block_place_writer: MessageWriter<BlockPlace>,
) {
    // 取出整段收件箱（move 出来，避免与后续可变借用冲突）。
    let inbox = std::mem::take(&mut clients.packet_inbox);
    for (conn_id, packet) in inbox {
        let Some(player) = connections.entity_of(conn_id) else {
            continue;
        };
        match packet {
            InboundPacket::InteractEntity(p) => {
                event_bus.dispatch(EntityInteract {
                    player,
                    target: p.target_id,
                    sneaking: p.sneaking,
                });
            }
            InboundPacket::PlayerAction(p) => {
                let _ = player_action_writer.write(PlayerActionEvent {
                    player,
                    status: p.status,
                    position: p.block_position,
                });
            }
            InboundPacket::Animation(p) => {
                event_bus.dispatch(PlayerAnimation {
                    player,
                    hand: p.hand,
                });
            }
            InboundPacket::UseItem(p) => {
                event_bus.dispatch(PlayerUseItem {
                    player,
                    hand: p.hand,
                });
            }
            InboundPacket::PlayerBlockPlacement(p) => {
                // 框架简化：仅派发 BlockPlace 事件（位置 + 占位空气方块）。
                // 放置的最终方块类型属原版逻辑，由应用侧决定；本框架不写实例。
                // 见 spec.md AI Amendment Log（T28）。
                let (x, y, z) = p.block_position;
                let _ = block_place_writer.write(BlockPlace {
                    position: Position::new(f64::from(x), f64::from(y), f64::from(z)),
                    block: crate::component::BlockState::from_id(0),
                });
            }
            InboundPacket::ClientPluginMessage(p) => {
                event_bus.dispatch(PluginMessageEvent {
                    player,
                    channel: p.channel,
                    data: p.data,
                    cancelled: false,
                });
            }
            InboundPacket::EditBook(p) => {
                event_bus.dispatch(BookEditEvent {
                    player,
                    hand: p.slot,
                    pages: p.pages,
                    cancelled: false,
                });
            }
            InboundPacket::ClickWindowButton(p) => {
                event_bus.dispatch(WindowButtonClickEvent {
                    player,
                    window_id: p.window_id as u8,
                    button_id: p.button_id,
                    cancelled: false,
                });
            }
            InboundPacket::CreativeInventoryAction(p) => {
                event_bus.dispatch(CreativeInventoryActionEvent {
                    player,
                    clicked_slot: p.slot as u8,
                    target_slot: p.slot as u8,
                    click_action: ClickAction::Other(0),
                    cancelled: false,
                });
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use crate::app::App;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::event::PlayerAnimation;
    use crate::event::bus::{EventContext, Listener};
    use crate::network::client::ClientNetworks;
    use crate::protocol::packets::play::Animation;
    use crate::protocol::packets::play::PlayerAction;
    use crate::protocol::packets::play::PlayerBlockPlacement;
    use crate::resource::connection_manager::ConnectionManager;

    /// 记录收到动画事件次数的监听器。
    #[derive(Clone)]
    struct AnimRecorder {
        count: Arc<Mutex<u32>>,
    }

    impl Listener<PlayerAnimation> for AnimRecorder {
        fn handle(&mut self, _ctx: &mut EventContext<PlayerAnimation>) {
            *self.count.lock().unwrap() += 1;
        }
    }

    /// 捕获最后一个 PluginMessageEvent 的监听器。
    #[derive(Clone)]
    struct PluginMsgRecorder {
        last: Arc<Mutex<Option<PluginMessageEvent>>>,
    }

    impl Listener<PluginMessageEvent> for PluginMsgRecorder {
        fn handle(&mut self, ctx: &mut EventContext<PluginMessageEvent>) {
            *self.last.lock().unwrap() = Some(ctx.event.clone());
        }
    }

    /// 捕获最后一个 BookEditEvent 的监听器。
    #[derive(Clone)]
    struct BookEditRecorder {
        last: Arc<Mutex<Option<BookEditEvent>>>,
    }

    impl Listener<BookEditEvent> for BookEditRecorder {
        fn handle(&mut self, ctx: &mut EventContext<BookEditEvent>) {
            *self.last.lock().unwrap() = Some(ctx.event.clone());
        }
    }

    /// 捕获最后一个 WindowButtonClickEvent 的监听器。
    #[derive(Clone)]
    struct WinBtnRecorder {
        last: Arc<Mutex<Option<WindowButtonClickEvent>>>,
    }

    impl Listener<WindowButtonClickEvent> for WinBtnRecorder {
        fn handle(&mut self, ctx: &mut EventContext<WindowButtonClickEvent>) {
            *self.last.lock().unwrap() = Some(ctx.event.clone());
        }
    }

    /// 捕获最后一个 CreativeInventoryActionEvent 的监听器。
    #[derive(Clone)]
    struct CreativeInvRecorder {
        last: Arc<Mutex<Option<CreativeInventoryActionEvent>>>,
    }

    impl Listener<CreativeInventoryActionEvent> for CreativeInvRecorder {
        fn handle(&mut self, ctx: &mut EventContext<CreativeInventoryActionEvent>) {
            *self.last.lock().unwrap() = Some(ctx.event.clone());
        }
    }

    #[test]
    fn packet_action_dispatches_animation_event() {
        let mut app = App::new();
        app.init_resource::<ClientNetworks>();
        app.init_resource::<ConnectionManager>();
        app.init_resource::<EventBus>();
        app.add_systems(packet_action_system);

        // 绑定玩家实体到 conn 1。
        let entity = app.world_mut().spawn_empty().id();
        app.world_mut()
            .resource_mut::<ConnectionManager>()
            .unwrap()
            .open(1, None)
            .entity = Some(entity);
        // 注册监听器。
        let recorder = AnimRecorder {
            count: Arc::new(Mutex::new(0)),
        };
        app.world_mut()
            .resource_mut::<EventBus>()
            .unwrap()
            .register::<PlayerAnimation>(recorder.clone());
        // 压入一个 Animation 动作包。
        app.world_mut()
            .resource_mut::<ClientNetworks>()
            .unwrap()
            .packet_inbox
            .push((1, InboundPacket::Animation(Animation { hand: 0 })));
        app.update();

        assert_eq!(*recorder.count.lock().unwrap(), 1);
    }

    #[test]
    fn packet_action_dispatches_plugin_message_event() {
        let mut app = App::new();
        app.init_resource::<ClientNetworks>();
        app.init_resource::<ConnectionManager>();
        app.init_resource::<EventBus>();
        app.add_systems(packet_action_system);

        let entity = app.world_mut().spawn_empty().id();
        app.world_mut()
            .resource_mut::<ConnectionManager>()
            .unwrap()
            .open(1, None)
            .entity = Some(entity);

        let recorder = PluginMsgRecorder {
            last: Arc::new(Mutex::new(None)),
        };
        app.world_mut()
            .resource_mut::<EventBus>()
            .unwrap()
            .register::<PluginMessageEvent>(recorder.clone());

        app.world_mut()
            .resource_mut::<ClientNetworks>()
            .unwrap()
            .packet_inbox
            .push((
                1,
                InboundPacket::ClientPluginMessage(
                    crate::protocol::packets::play::ClientPluginMessage {
                        channel: "minecraft:brand".to_string(),
                        data: vec![1, 2, 3],
                    },
                ),
            ));
        app.update();

        let event = recorder.last.lock().unwrap().take().unwrap();
        assert_eq!(event.player, entity);
        assert_eq!(event.channel, "minecraft:brand");
        assert_eq!(event.data, vec![1, 2, 3]);
        assert!(!event.cancelled);
    }

    #[test]
    fn packet_action_dispatches_book_edit_event() {
        let mut app = App::new();
        app.init_resource::<ClientNetworks>();
        app.init_resource::<ConnectionManager>();
        app.init_resource::<EventBus>();
        app.add_systems(packet_action_system);

        let entity = app.world_mut().spawn_empty().id();
        app.world_mut()
            .resource_mut::<ConnectionManager>()
            .unwrap()
            .open(1, None)
            .entity = Some(entity);

        let recorder = BookEditRecorder {
            last: Arc::new(Mutex::new(None)),
        };
        app.world_mut()
            .resource_mut::<EventBus>()
            .unwrap()
            .register::<BookEditEvent>(recorder.clone());

        app.world_mut()
            .resource_mut::<ClientNetworks>()
            .unwrap()
            .packet_inbox
            .push((
                1,
                InboundPacket::EditBook(crate::protocol::packets::play::EditBook {
                    slot: 1,
                    pages: vec!["page one".to_string(), "page two".to_string()],
                    title: Some("My Book".to_string()),
                }),
            ));
        app.update();

        let event = recorder.last.lock().unwrap().take().unwrap();
        assert_eq!(event.player, entity);
        assert_eq!(event.hand, 1);
        assert_eq!(
            event.pages,
            vec!["page one".to_string(), "page two".to_string()]
        );
        assert!(!event.cancelled);
    }

    #[test]
    fn packet_action_dispatches_window_button_click_event() {
        let mut app = App::new();
        app.init_resource::<ClientNetworks>();
        app.init_resource::<ConnectionManager>();
        app.init_resource::<EventBus>();
        app.add_systems(packet_action_system);

        let entity = app.world_mut().spawn_empty().id();
        app.world_mut()
            .resource_mut::<ConnectionManager>()
            .unwrap()
            .open(1, None)
            .entity = Some(entity);

        let recorder = WinBtnRecorder {
            last: Arc::new(Mutex::new(None)),
        };
        app.world_mut()
            .resource_mut::<EventBus>()
            .unwrap()
            .register::<WindowButtonClickEvent>(recorder.clone());

        app.world_mut()
            .resource_mut::<ClientNetworks>()
            .unwrap()
            .packet_inbox
            .push((
                1,
                InboundPacket::ClickWindowButton(
                    crate::protocol::packets::play::ClickWindowButton {
                        window_id: 5,
                        button_id: 3,
                    },
                ),
            ));
        app.update();

        let event = recorder.last.lock().unwrap().take().unwrap();
        assert_eq!(event.player, entity);
        assert_eq!(event.window_id, 5);
        assert_eq!(event.button_id, 3);
        assert!(!event.cancelled);
    }

    #[test]
    fn packet_action_dispatches_creative_inventory_action_event() {
        let mut app = App::new();
        app.init_resource::<ClientNetworks>();
        app.init_resource::<ConnectionManager>();
        app.init_resource::<EventBus>();
        app.add_systems(packet_action_system);

        let entity = app.world_mut().spawn_empty().id();
        app.world_mut()
            .resource_mut::<ConnectionManager>()
            .unwrap()
            .open(1, None)
            .entity = Some(entity);

        let recorder = CreativeInvRecorder {
            last: Arc::new(Mutex::new(None)),
        };
        app.world_mut()
            .resource_mut::<EventBus>()
            .unwrap()
            .register::<CreativeInventoryActionEvent>(recorder.clone());

        app.world_mut()
            .resource_mut::<ClientNetworks>()
            .unwrap()
            .packet_inbox
            .push((
                1,
                InboundPacket::CreativeInventoryAction(
                    crate::protocol::packets::play::CreativeInventoryAction {
                        slot: 36,
                        item: crate::item_stack::ItemStack::AIR,
                    },
                ),
            ));
        app.update();

        let event = recorder.last.lock().unwrap().take().unwrap();
        assert_eq!(event.player, entity);
        assert_eq!(event.clicked_slot, 36);
        assert_eq!(event.target_slot, 36);
        assert_eq!(event.click_action, ClickAction::Other(0));
        assert!(!event.cancelled);
    }

    /// 记录 PlayerActionEvent 被接收顺序的监听器。
    #[derive(Clone)]
    struct ActionEventRecorder {
        log: Arc<Mutex<Vec<(u32, i32, i32)>>>,
        id: u32,
    }

    impl Listener<PlayerActionEvent> for ActionEventRecorder {
        fn handle(&mut self, ctx: &mut EventContext<PlayerActionEvent>) {
            let e = &ctx.event;
            self.log.lock().unwrap().push((self.id, e.status, e.position.0));
        }
    }

    /// 记录 BlockPlace 被接收顺序的监听器。
    #[derive(Clone)]
    struct BlockPlaceRecorder {
        log: Arc<Mutex<Vec<(u32, i32, i32, i32)>>>,
        id: u32,
    }

    impl Listener<BlockPlace> for BlockPlaceRecorder {
        fn handle(&mut self, ctx: &mut EventContext<BlockPlace>) {
            let pos = ctx.event.position;
            self.log.lock().unwrap().push((
                self.id,
                pos.x.ceil() as i32,
                pos.y.ceil() as i32,
                pos.z.ceil() as i32,
            ));
        }
    }

    /// 验证多个 PlayerActionEvent 一次性批量派发时，监听器按注册顺序被调用。
    #[test]
    fn packet_action_dispatches_player_action_events_in_batch() {
        let mut app = App::new();
        app.init_resource::<ClientNetworks>();
        app.init_resource::<ConnectionManager>();
        app.init_resource::<crate::event::bus::EventBus>();
        app.init_resource::<crate::instance::chunk_store::ChunkStore>();
        app.insert_resource::<crate::resource::registries::BlockRegistry>(
            crate::resource::registries::BlockRegistry::default(),
        );
        app.add_systems(packet_action_system);
        app.add_systems(super::super::block_interaction_validator::block_interaction_validator);

        let entity = app.world_mut().spawn_empty()
            .insert(crate::component::Position::new(5.0, 64.0, 25.0))
            .insert(crate::component::Player::new(
                uuid::Uuid::new_v4(),
                "test",
            ))
            .id();
        app.world_mut()
            .resource_mut::<ConnectionManager>()
            .unwrap()
            .open(1, None)
            .entity = Some(entity);

        // 在目标区块加载石头方块（不透明，light_opacity=15）。
        // 加载 chunk(0,0) 以覆盖玩家 (5,64,25) 及附近作用点。
        let chunk = {
            let mut c = crate::instance::chunk::Chunk::new(0, 0, 1);
            for i in 0..crate::instance::chunk::SECTION_VOLUME {
                let _ = c.set_block(0, i, 1);
            }
            c
        };
        app.world_mut()
            .resource_mut::<crate::instance::chunk_store::ChunkStore>()
            .unwrap()
            .load_chunk(chunk);

        let log = Arc::new(Mutex::new(Vec::new()));
        let bus = app.world_mut().resource_mut::<EventBus>().unwrap();
        bus.register(ActionEventRecorder {
            log: log.clone(),
            id: 1,
        });
        bus.register(ActionEventRecorder {
            log: log.clone(),
            id: 2,
        });

        // 压入两条 PlayerAction 包。
        // 位置均在 chunk(0,0) 内且距离玩家 (5,64,25) ≤ 6 格。
        app.world_mut()
            .resource_mut::<ClientNetworks>()
            .unwrap()
            .packet_inbox
            .extend([
                (
                    1,
                    InboundPacket::PlayerAction(PlayerAction {
                        status: 1,
                        block_position: (5, 64, 20),
                        block_face: 0,
                        sequence: 0,
                    }),
                ),
                (
                    1,
                    InboundPacket::PlayerAction(PlayerAction {
                        status: 2,
                        block_position: (8, 64, 25),
                        block_face: 0,
                        sequence: 1,
                    }),
                ),
            ]);
        app.update();

        // status=1（取消挖掘）直接放行；status=2 需通过校验（距离+区块已加载），
        // 两条事件各触发两个监听器，顺序为 [1, 2, 1, 2]。
        let records = log.lock().unwrap();
        assert_eq!(records.len(), 4);
        assert_eq!(records[0], (1, 1, 5));
        assert_eq!(records[1], (2, 1, 5));
        assert_eq!(records[2], (1, 2, 8));
        assert_eq!(records[3], (2, 2, 8));
    }

    /// 验证多个 BlockPlace 一次性批量派发时，监听器按注册顺序被调用。
    #[test]
    fn packet_action_dispatches_block_place_events_in_batch() {
        let mut app = App::new();
        app.init_resource::<ClientNetworks>();
        app.init_resource::<ConnectionManager>();
        app.init_resource::<crate::event::bus::EventBus>();
        app.init_resource::<crate::instance::chunk_store::ChunkStore>();
        app.insert_resource::<crate::resource::registries::BlockRegistry>(
            crate::resource::registries::BlockRegistry::default(),
        );
        app.add_systems(packet_action_system);
        app.add_systems(super::super::block_interaction_validator::block_interaction_validator);

        let entity = app.world_mut().spawn_empty()
            .insert(crate::component::Position::new(5.0, 64.0, 10.0))
            .insert(crate::component::Player::new(
                uuid::Uuid::new_v4(),
                "test",
            ))
            .id();
        app.world_mut()
            .resource_mut::<ConnectionManager>()
            .unwrap()
            .open(1, None)
            .entity = Some(entity);

        let log = Arc::new(Mutex::new(Vec::new()));
        let bus = app.world_mut().resource_mut::<EventBus>().unwrap();
        bus.register(BlockPlaceRecorder {
            log: log.clone(),
            id: 1,
        });
        bus.register(BlockPlaceRecorder {
            log: log.clone(),
            id: 2,
        });

        // 压入两条 PlayerBlockPlacement 包。两个位置均在玩家 (5,64,10) 的 6 格距离内，
        // 且都落在已加载的 chunk(0,0) 范围内。
        app.world_mut()
            .resource_mut::<ClientNetworks>()
            .unwrap()
            .packet_inbox
            .extend([
                (
                    1,
                    InboundPacket::PlayerBlockPlacement(PlayerBlockPlacement {
                        hand: 0,
                        block_position: (5, 64, 10),
                        block_face: 1,
                        cursor_position_x: 0.5,
                        cursor_position_y: 0.5,
                        cursor_position_z: 0.5,
                        inside_block: false,
                        hit_world_border: false,
                        sequence: 0,
                    }),
                ),
                (
                    1,
                    InboundPacket::PlayerBlockPlacement(PlayerBlockPlacement {
                        hand: 0,
                        block_position: (8, 64, 13),
                        block_face: 1,
                        cursor_position_x: 0.5,
                        cursor_position_y: 0.5,
                        cursor_position_z: 0.5,
                        inside_block: false,
                        hit_world_border: false,
                        sequence: 1,
                    }),
                ),
            ]);
        app.update();

        // 两条 BlockPlace 事件均通过距离校验（max dist=6），各触发两个监听器，顺序为 [1, 2, 1, 2]。
        let records = log.lock().unwrap();
        assert_eq!(records.len(), 4);
        assert_eq!(records[0], (1, 5, 64, 10));
        assert_eq!(records[1], (2, 5, 64, 10));
        assert_eq!(records[2], (1, 8, 64, 13));
        assert_eq!(records[3], (2, 8, 64, 13));
    }
}
