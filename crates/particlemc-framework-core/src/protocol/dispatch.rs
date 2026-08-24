//! 按 `(ConnectionState, packet_id)` 派发入站包。

use crate::network::connection::ConnectionState;
use crate::protocol::byte_buf::ByteBuffer;
use crate::protocol::error::ProtocolError;
use crate::protocol::packet::Packet;
use crate::protocol::packets::{InboundPacket, configuration, handshake, login, play, status};

/// 将一帧的包体（不含 packet_id）按状态与 id 派发为 [`InboundPacket`]。
///
/// - 握手 / 状态 / 登录 / 配置阶段出现超出已知范围的 id → 返回
///   [`ProtocolError::UnknownPacket`]，由监听任务记录后跳过（不 panic）。
/// - 游玩阶段大量包本框架不处理，未匹配 id 返回 [`InboundPacket::Ignored`]（静默忽略）。
pub fn dispatch(
    state: ConnectionState,
    packet_id: i32,
    payload: &[u8],
) -> Result<InboundPacket, ProtocolError> {
    let mut buf = ByteBuffer::new(payload.to_vec());
    match state {
        ConnectionState::Handshake => match packet_id {
            0x00 => Ok(InboundPacket::Intention(handshake::Intention::decode(
                &mut buf,
            )?)),
            _ => Err(ProtocolError::UnknownPacket {
                state: state.state_name(),
                id: packet_id,
            }),
        },
        ConnectionState::Status => match packet_id {
            0x00 => Ok(InboundPacket::StatusRequest),
            0x01 => Ok(InboundPacket::Ping(status::Ping::decode(&mut buf)?)),
            _ => Err(ProtocolError::UnknownPacket {
                state: state.state_name(),
                id: packet_id,
            }),
        },
        ConnectionState::Login => match packet_id {
            0x00 => Ok(InboundPacket::Hello(login::Hello::decode(&mut buf)?)),
            0x01 => Ok(InboundPacket::LoginChallenge(
                login::LoginChallenge::decode(&mut buf)?,
            )),
            0x02 => Ok(InboundPacket::Ignored { packet_id }),
            0x03 => Ok(InboundPacket::LoginAcknowledged),
            _ => Err(ProtocolError::UnknownPacket {
                state: state.state_name(),
                id: packet_id,
            }),
        },
        ConnectionState::Configuration => match packet_id {
            0x00 => Ok(InboundPacket::ClientInformation(
                configuration::ClientInformation,
            )),
            0x01 | 0x02 | 0x04 | 0x05 | 0x06 => Ok(InboundPacket::Ignored { packet_id }),
            0x03 => Ok(InboundPacket::FinishConfiguration),
            _ => Err(ProtocolError::UnknownPacket {
                state: state.state_name(),
                id: packet_id,
            }),
        },
        ConnectionState::Play => match packet_id {
            0x00 => Ok(InboundPacket::TeleportConfirm(
                play::TeleportConfirm::decode(&mut buf)?,
            )),
            // 容器点击（Play, id 0x11）：权威重算玩家库存，详见 `.specs/implement-item-click/`。
            0x11 => Ok(InboundPacket::ClickContainer(play::ClickContainer::decode(
                &mut buf,
            )?)),
            // 关闭容器（Play, id 0x12）：清空玩家光标，详见 `.specs/implement-item-inventory/`。
            0x12 => Ok(InboundPacket::CloseContainer(play::CloseContainer::decode(
                &mut buf,
            )?)),
            // 手持切换（Play, id 0x34）：ClientHeldItemChange，详见 `.specs/implement-item-inventory/`。
            0x34 => Ok(InboundPacket::HeldItemChange(
                play::ClientHeldItemChange::decode(&mut buf)?,
            )),
            0x0a => Ok(InboundPacket::ChunkBatchReceived(
                play::ChunkBatchReceived::decode(&mut buf)?,
            )),
            // 客户端状态（Play, id 0x0b，wire 名 `client_status`）：请求重生 / 统计。
            // 0x0b 权威映射为 `ClientStatusPacket`（修复历史错配，见
            // `.specs/implement-framework-capabilities/`）。
            0x0b => Ok(InboundPacket::ClientStatus(play::Status::decode(&mut buf)?)),
            // 命令聊天（Play, id 0x06）：含 "/" 前缀命令或无前缀指令文本，见
            // `.specs/implement-command-framework/`。
            0x06 => Ok(InboundPacket::ClientCommandChat(
                play::ClientCommandChatPacket::decode(&mut buf)?,
            )),
            // 签名命令聊天（Play, id 0x07）：签名版命令文本，见
            // `.specs/implement-command-framework/`。
            0x07 => Ok(InboundPacket::ClientSignedCommandChat(
                play::ClientSignedCommandChatPacket::decode(&mut buf)?,
            )),
            0x1b => Ok(InboundPacket::KeepAlive(play::KeepAlive::decode(&mut buf)?)),
            0x1d => Ok(InboundPacket::PlayerPosition(play::PlayerPosition::decode(
                &mut buf,
            )?)),
            0x1e => Ok(InboundPacket::PlayerPositionAndRotation(
                play::PlayerPositionAndRotation::decode(&mut buf)?,
            )),
            0x1f => Ok(InboundPacket::Look(play::Look::decode(&mut buf)?)),
            0x20 => Ok(InboundPacket::PlayerPositionStatus(
                play::PlayerPositionStatus::decode(&mut buf)?,
            )),
            0x2b => Ok(InboundPacket::PlayerLoaded(play::PlayerLoaded::decode(
                &mut buf,
            )?)),
            0x01 => Ok(InboundPacket::QueryBlockNbt(play::QueryBlockNbt::decode(
                &mut buf,
            )?)),
            0x02 => Ok(InboundPacket::SelectBundleItem(
                play::SelectBundleItem::decode(&mut buf)?,
            )),
            0x03 => Ok(InboundPacket::ChangeDifficulty(
                play::ChangeDifficulty::decode(&mut buf)?,
            )),
            0x04 => Ok(InboundPacket::ChangeGameMode(play::ChangeGameMode::decode(
                &mut buf,
            )?)),
            0x05 => Ok(InboundPacket::ChatAck(play::ChatAck::decode(&mut buf)?)),
            0x08 => Ok(InboundPacket::ChatMessage(play::ChatMessage::decode(
                &mut buf,
            )?)),
            0x09 => Ok(InboundPacket::ChatSessionUpdate(
                play::ChatSessionUpdate::decode(&mut buf)?,
            )),
            0x0c => Ok(InboundPacket::TickEnd(play::TickEnd::decode(&mut buf)?)),
            0x0d => Ok(InboundPacket::Settings(play::Settings::decode(&mut buf)?)),
            0x0e => Ok(InboundPacket::TabComplete(play::TabComplete::decode(
                &mut buf,
            )?)),
            0x0f => Ok(InboundPacket::ConfigurationAck(
                play::ConfigurationAck::decode(&mut buf)?,
            )),
            0x10 => Ok(InboundPacket::ClickWindowButton(
                play::ClickWindowButton::decode(&mut buf)?,
            )),
            0x13 => Ok(InboundPacket::WindowSlotState(
                play::WindowSlotState::decode(&mut buf)?,
            )),
            0x14 => Ok(InboundPacket::CookieResponse(play::CookieResponse::decode(
                &mut buf,
            )?)),
            0x15 => Ok(InboundPacket::ClientPluginMessage(
                play::ClientPluginMessage::decode(&mut buf)?,
            )),
            0x16 => Ok(InboundPacket::DebugSubscriptionRequest(
                play::DebugSubscriptionRequest::decode(&mut buf)?,
            )),
            0x17 => Ok(InboundPacket::EditBook(play::EditBook::decode(&mut buf)?)),
            0x18 => Ok(InboundPacket::QueryEntityNbt(play::QueryEntityNbt::decode(
                &mut buf,
            )?)),
            0x19 => Ok(InboundPacket::InteractEntity(play::InteractEntity::decode(
                &mut buf,
            )?)),
            0x1a => Ok(InboundPacket::GenerateStructure(
                play::GenerateStructure::decode(&mut buf)?,
            )),
            0x1c => Ok(InboundPacket::LockDifficulty(play::LockDifficulty::decode(
                &mut buf,
            )?)),
            0x21 => Ok(InboundPacket::VehicleMove(play::VehicleMove::decode(
                &mut buf,
            )?)),
            0x22 => Ok(InboundPacket::SteerBoat(play::SteerBoat::decode(&mut buf)?)),
            0x23 => Ok(InboundPacket::PickItemFromBlock(
                play::PickItemFromBlock::decode(&mut buf)?,
            )),
            0x24 => Ok(InboundPacket::PickItemFromEntity(
                play::PickItemFromEntity::decode(&mut buf)?,
            )),
            0x25 => Ok(InboundPacket::PingRequest(play::PingRequest::decode(
                &mut buf,
            )?)),
            0x26 => Ok(InboundPacket::PlaceRecipe(play::PlaceRecipe::decode(
                &mut buf,
            )?)),
            0x27 => Ok(InboundPacket::PlayerAbilities(
                play::PlayerAbilities::decode(&mut buf)?,
            )),
            0x28 => Ok(InboundPacket::PlayerAction(play::PlayerAction::decode(
                &mut buf,
            )?)),
            0x29 => Ok(InboundPacket::EntityAction(play::EntityAction::decode(
                &mut buf,
            )?)),
            0x2a => Ok(InboundPacket::Input(play::Input::decode(&mut buf)?)),
            0x2c => Ok(InboundPacket::Pong(play::Pong::decode(&mut buf)?)),
            0x2d => Ok(InboundPacket::SetRecipeBookState(
                play::SetRecipeBookState::decode(&mut buf)?,
            )),
            0x2e => Ok(InboundPacket::RecipeBookSeenRecipe(
                play::RecipeBookSeenRecipe::decode(&mut buf)?,
            )),
            0x2f => Ok(InboundPacket::NameItem(play::NameItem::decode(&mut buf)?)),
            0x30 => Ok(InboundPacket::ResourcePackStatus(
                play::ResourcePackStatus::decode(&mut buf)?,
            )),
            0x31 => Ok(InboundPacket::AdvancementTab(play::AdvancementTab::decode(
                &mut buf,
            )?)),
            0x32 => Ok(InboundPacket::SelectTrade(play::SelectTrade::decode(
                &mut buf,
            )?)),
            0x33 => Ok(InboundPacket::SetBeaconEffect(
                play::SetBeaconEffect::decode(&mut buf)?,
            )),
            0x35 => Ok(InboundPacket::UpdateCommandBlock(
                play::UpdateCommandBlock::decode(&mut buf)?,
            )),
            0x36 => Ok(InboundPacket::UpdateCommandBlockMinecart(
                play::UpdateCommandBlockMinecart::decode(&mut buf)?,
            )),
            0x37 => Ok(InboundPacket::CreativeInventoryAction(
                play::CreativeInventoryAction::decode(&mut buf)?,
            )),
            0x38 => Ok(InboundPacket::UpdateJigsawBlock(
                play::UpdateJigsawBlock::decode(&mut buf)?,
            )),
            0x39 => Ok(InboundPacket::UpdateStructureBlock(
                play::UpdateStructureBlock::decode(&mut buf)?,
            )),
            0x3a => Ok(InboundPacket::SetTestBlock(play::SetTestBlock::decode(
                &mut buf,
            )?)),
            0x3b => Ok(InboundPacket::UpdateSign(play::UpdateSign::decode(
                &mut buf,
            )?)),
            0x3c => Ok(InboundPacket::Animation(play::Animation::decode(&mut buf)?)),
            0x3d => Ok(InboundPacket::Spectate(play::Spectate::decode(&mut buf)?)),
            0x3e => Ok(InboundPacket::TestInstanceBlockAction(
                play::TestInstanceBlockAction::decode(&mut buf)?,
            )),
            0x3f => Ok(InboundPacket::PlayerBlockPlacement(
                play::PlayerBlockPlacement::decode(&mut buf)?,
            )),
            0x40 => Ok(InboundPacket::UseItem(play::UseItem::decode(&mut buf)?)),
            0x41 => Ok(InboundPacket::CustomClickAction(
                play::CustomClickAction::decode(&mut buf)?,
            )),
            _ => Ok(InboundPacket::Ignored { packet_id }),
        },
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::item_stack::ItemStack;
    use crate::protocol::byte_buf::ByteBuffer;
    use crate::protocol::packets::ClickContainer;
    use uuid::Uuid;

    /// 将包编码为纯包体字节（不含 packet_id），供 dispatch 消费。
    fn payload_of<P: Packet>(p: &P) -> Vec<u8> {
        let mut buf = ByteBuffer::with_capacity(64);
        p.encode(&mut buf).unwrap();
        buf.into_inner()
    }

    #[test]
    fn handshake_0x00_dispatches_intention() {
        // 构造并编码 Handshake Intention，派发后应还原出相同字段。
        let pkt = handshake::Intention {
            protocol_version: 774,
            server_address: "localhost".to_string(),
            port: 25565,
            next_state: 2,
            forwarding: None,
        };
        let out = dispatch(ConnectionState::Handshake, 0x00, &payload_of(&pkt)).unwrap();
        match out {
            InboundPacket::Intention(i) => assert_eq!(i, pkt),
            other => panic!("期望 Intention，实际 {other:?}"),
        }
    }

    #[test]
    fn status_0x00_dispatches_status_request() {
        // StatusRequest 无字段，空包体即可。
        let out = dispatch(ConnectionState::Status, 0x00, &[]).unwrap();
        assert!(matches!(out, InboundPacket::StatusRequest));
    }

    #[test]
    fn status_0x01_dispatches_ping() {
        let pkt = status::Ping {
            payload: 123_456_789,
        };
        let out = dispatch(ConnectionState::Status, 0x01, &payload_of(&pkt)).unwrap();
        match out {
            InboundPacket::Ping(p) => assert_eq!(p, pkt),
            other => panic!("期望 Ping，实际 {other:?}"),
        }
    }

    #[test]
    fn login_0x00_dispatches_hello() {
        // Hello payload = String + bool + uuid（有 uuid 场景）。
        let pkt = login::Hello {
            name: "Steve".to_string(),
            uuid: Some(Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef)),
        };
        let out = dispatch(ConnectionState::Login, 0x00, &payload_of(&pkt)).unwrap();
        match out {
            InboundPacket::Hello(h) => assert_eq!(h, pkt),
            other => panic!("期望 Hello，实际 {other:?}"),
        }
    }

    #[test]
    fn login_0x03_dispatches_login_acknowledged() {
        // LoginAcknowledged 无字段，空包体即可。
        let out = dispatch(ConnectionState::Login, 0x03, &[]).unwrap();
        assert!(matches!(out, InboundPacket::LoginAcknowledged));
    }

    #[test]
    fn configuration_0x00_dispatches_client_information() {
        // ClientInformation 无字段，空包体即可。
        let out = dispatch(ConnectionState::Configuration, 0x00, &[]).unwrap();
        assert!(matches!(out, InboundPacket::ClientInformation(_)));
    }

    #[test]
    fn configuration_0x03_dispatches_finish_configuration() {
        let out = dispatch(ConnectionState::Configuration, 0x03, &[]).unwrap();
        assert!(matches!(out, InboundPacket::FinishConfiguration));
    }

    #[test]
    fn play_0x1d_dispatches_player_position() {
        let pkt = play::PlayerPosition {
            x: 1.0,
            y: 64.0,
            z: -2.5,
            grounded: true,
        };
        let out = dispatch(ConnectionState::Play, 0x1d, &payload_of(&pkt)).unwrap();
        match out {
            InboundPacket::PlayerPosition(p) => assert_eq!(p, pkt),
            other => panic!("期望 PlayerPosition，实际 {other:?}"),
        }
    }

    #[test]
    fn play_player_position_manual_payload() {
        // 手工构造 3×f64 + bool = 25 字节的包体，验证派发解码正确。
        let mut raw = Vec::new();
        raw.extend_from_slice(&1.0f64.to_be_bytes());
        raw.extend_from_slice(&64.0f64.to_be_bytes());
        raw.extend_from_slice(&0.0f64.to_be_bytes());
        raw.push(1u8); // grounded = true
        assert_eq!(raw.len(), 25);

        let out = dispatch(ConnectionState::Play, 0x1d, &raw).unwrap();
        match out {
            InboundPacket::PlayerPosition(p) => {
                assert_eq!(p.x, 1.0);
                assert_eq!(p.y, 64.0);
                assert_eq!(p.z, 0.0);
                assert!(p.grounded);
            }
            other => panic!("期望 PlayerPosition，实际 {other:?}"),
        }
    }

    #[test]
    fn play_0x1e_dispatches_player_position_and_rotation() {
        let pkt = play::PlayerPositionAndRotation {
            x: 1.0,
            y: 64.0,
            z: -2.5,
            yaw: 10.0,
            pitch: -5.0,
            grounded: false,
        };
        let out = dispatch(ConnectionState::Play, 0x1e, &payload_of(&pkt)).unwrap();
        match out {
            InboundPacket::PlayerPositionAndRotation(p) => assert_eq!(p, pkt),
            other => panic!("期望 PlayerPositionAndRotation，实际 {other:?}"),
        }
    }

    #[test]
    fn play_0x0b_dispatches_client_status() {
        let pkt = play::Status { action: 0 }; // PERFORM_RESPAWN
        let out = dispatch(ConnectionState::Play, 0x0b, &payload_of(&pkt)).unwrap();
        match out {
            InboundPacket::ClientStatus(p) => assert_eq!(p, pkt),
            other => panic!("期望 ClientStatus，实际 {other:?}"),
        }
    }

    #[test]
    fn play_0x06_dispatches_client_command_chat() {
        let pkt = play::ClientCommandChatPacket {
            message: "help".to_string(),
        };
        let out = dispatch(ConnectionState::Play, 0x06, &payload_of(&pkt)).unwrap();
        match out {
            InboundPacket::ClientCommandChat(p) => assert_eq!(p, pkt),
            other => panic!("期望 ClientCommandChat，实际 {other:?}"),
        }
    }

    #[test]
    fn play_0x07_dispatches_client_signed_command_chat() {
        let pkt = play::ClientSignedCommandChatPacket {
            message: "say hi".to_string(),
        };
        let out = dispatch(ConnectionState::Play, 0x07, &payload_of(&pkt)).unwrap();
        match out {
            InboundPacket::ClientSignedCommandChat(p) => assert_eq!(p, pkt),
            other => panic!("期望 ClientSignedCommandChat，实际 {other:?}"),
        }
    }

    #[test]
    fn play_0x06_not_ignored() {
        let pkt = play::ClientCommandChatPacket {
            message: "gamemode".to_string(),
        };
        let out = dispatch(ConnectionState::Play, 0x06, &payload_of(&pkt)).unwrap();
        assert!(
            !matches!(out, InboundPacket::Ignored { .. }),
            "0x06 不应被忽略，实际 {out:?}"
        );
    }

    #[test]
    fn play_0x07_not_ignored() {
        let pkt = play::ClientSignedCommandChatPacket {
            message: "gamemode".to_string(),
        };
        let out = dispatch(ConnectionState::Play, 0x07, &payload_of(&pkt)).unwrap();
        assert!(
            !matches!(out, InboundPacket::Ignored { .. }),
            "0x07 不应被忽略，实际 {out:?}"
        );
    }

    #[test]
    fn play_0x08_dispatches_chat_message() {
        // 0x08 现为 ChatMessage（网络全面补全后不再 Ignored）。
        let pkt = play::ChatMessage {
            message: "hello".to_string(),
            timestamp: 0,
            salt: 0,
            signature: None,
            ack_offset: 0,
            ack_list: [0, 0, 0],
            checksum: 0,
        };
        let out = dispatch(ConnectionState::Play, 0x08, &payload_of(&pkt)).unwrap();
        match out {
            InboundPacket::ChatMessage(p) => assert_eq!(p, pkt),
            other => panic!("期望 ChatMessage，实际 {other:?}"),
        }
    }

    #[test]
    fn play_0x00_dispatches_teleport_confirm() {
        let pkt = play::TeleportConfirm { teleport_id: 7 };
        let out = dispatch(ConnectionState::Play, 0x00, &payload_of(&pkt)).unwrap();
        match out {
            InboundPacket::TeleportConfirm(p) => assert_eq!(p, pkt),
            other => panic!("期望 TeleportConfirm，实际 {other:?}"),
        }
    }

    #[test]
    fn play_0x0a_dispatches_chunk_batch_received() {
        let pkt = play::ChunkBatchReceived {
            chunks_per_tick: 20.0,
        };
        let out = dispatch(ConnectionState::Play, 0x0a, &payload_of(&pkt)).unwrap();
        match out {
            InboundPacket::ChunkBatchReceived(p) => assert_eq!(p, pkt),
            other => panic!("期望 ChunkBatchReceived，实际 {other:?}"),
        }
    }

    #[test]
    fn play_0x11_dispatches_click_container() {
        // 构造容器点击包（见 `.specs/implement-item-click/`），派发后应还原字段。
        let pkt = ClickContainer {
            window_id: 0,
            state_id: 0,
            slot: 36,
            button: 0,
            mode: 0,
            changed_slots: vec![],
            carried_item: ItemStack::AIR,
        };
        let out = dispatch(ConnectionState::Play, 0x11, &payload_of(&pkt)).unwrap();
        match out {
            InboundPacket::ClickContainer(c) => {
                assert_eq!(c.window_id, 0);
                assert_eq!(c.state_id, 0);
                assert_eq!(c.slot, 36);
                assert_eq!(c.button, 0);
                assert_eq!(c.mode, 0);
                assert!(c.changed_slots.is_empty());
                assert_eq!(c.carried_item, ItemStack::AIR);
            }
            other => panic!("期望 ClickContainer，实际 {other:?}"),
        }
    }

    #[test]
    fn play_0x11_not_ignored() {
        // 0x11 已接入派发，不应再落到 Ignored。
        let pkt = ClickContainer {
            window_id: 0,
            state_id: 0,
            slot: 36,
            button: 0,
            mode: 0,
            changed_slots: vec![],
            carried_item: ItemStack::AIR,
        };
        let out = dispatch(ConnectionState::Play, 0x11, &payload_of(&pkt)).unwrap();
        assert!(
            !matches!(out, InboundPacket::Ignored { .. }),
            "0x11 不应被忽略，实际 {out:?}"
        );
    }

    #[test]
    fn play_0x1b_dispatches_keep_alive() {
        let pkt = play::KeepAlive {
            keep_alive_id: 0x1234_5678_9abc_def0,
        };
        let out = dispatch(ConnectionState::Play, 0x1b, &payload_of(&pkt)).unwrap();
        match out {
            InboundPacket::KeepAlive(p) => assert_eq!(p, pkt),
            other => panic!("期望 KeepAlive，实际 {other:?}"),
        }
    }

    #[test]
    fn play_0x1f_dispatches_look() {
        let pkt = play::Look {
            yaw: 90.0,
            pitch: -10.0,
            on_ground: true,
        };
        let out = dispatch(ConnectionState::Play, 0x1f, &payload_of(&pkt)).unwrap();
        match out {
            InboundPacket::Look(p) => assert_eq!(p, pkt),
            other => panic!("期望 Look，实际 {other:?}"),
        }
    }

    #[test]
    fn play_0x20_dispatches_player_position_status() {
        let pkt = play::PlayerPositionStatus {
            flags: play::PlayerPositionStatus::FLAG_ON_GROUND,
        };
        let out = dispatch(ConnectionState::Play, 0x20, &payload_of(&pkt)).unwrap();
        match out {
            InboundPacket::PlayerPositionStatus(p) => assert_eq!(p, pkt),
            other => panic!("期望 PlayerPositionStatus，实际 {other:?}"),
        }
    }

    #[test]
    fn play_0x2b_dispatches_player_loaded() {
        let out = dispatch(ConnectionState::Play, 0x2b, &[]).unwrap();
        assert!(matches!(out, InboundPacket::PlayerLoaded(_)));
    }

    #[test]
    fn unknown_packet_id_returns_err_in_login() {
        // Login 阶段未定义 0xFF → UnknownPacket，而非 panic。
        let err = dispatch(ConnectionState::Login, 0xFF, &[]).unwrap_err();
        assert!(
            matches!(err, ProtocolError::UnknownPacket { .. }),
            "期望 UnknownPacket，实际 {err:?}"
        );
    }

    #[test]
    fn unknown_packet_id_returns_err_in_configuration() {
        // Configuration 阶段未定义 0xFF → UnknownPacket，而非 panic。
        let err = dispatch(ConnectionState::Configuration, 0xFF, &[]).unwrap_err();
        assert!(
            matches!(err, ProtocolError::UnknownPacket { .. }),
            "期望 UnknownPacket，实际 {err:?}"
        );
    }

    #[test]
    fn play_unmatched_id_is_ignored() {
        // Play 阶段未匹配 id（如 0x7F）→ 静默忽略，不报错。
        let out = dispatch(ConnectionState::Play, 0x7F, &[]).unwrap();
        assert!(matches!(out, InboundPacket::Ignored { packet_id: 0x7F }));
    }

    #[test]
    fn play_0x12_dispatches_close_container() {
        // 关闭容器（见 `.specs/implement-item-inventory/`），派发后应还原 window_id。
        let pkt = play::CloseContainer { window_id: 0 };
        let out = dispatch(ConnectionState::Play, 0x12, &payload_of(&pkt)).unwrap();
        match out {
            InboundPacket::CloseContainer(c) => assert_eq!(c.window_id, 0),
            other => panic!("期望 CloseContainer，实际 {other:?}"),
        }
    }

    #[test]
    fn play_0x12_not_ignored() {
        // 0x12 已接入派发，不应再落到 Ignored。
        let pkt = play::CloseContainer { window_id: 0 };
        let out = dispatch(ConnectionState::Play, 0x12, &payload_of(&pkt)).unwrap();
        assert!(
            !matches!(out, InboundPacket::Ignored { .. }),
            "0x12 不应被忽略，实际 {out:?}"
        );
    }

    #[test]
    fn play_0x34_dispatches_held_item_change() {
        // 手持切换（见 `.specs/implement-item-inventory/`），派发后应还原 slot。
        let pkt = play::ClientHeldItemChange { slot: 3 };
        let out = dispatch(ConnectionState::Play, 0x34, &payload_of(&pkt)).unwrap();
        match out {
            InboundPacket::HeldItemChange(c) => assert_eq!(c.slot, 3),
            other => panic!("期望 HeldItemChange，实际 {other:?}"),
        }
    }

    #[test]
    fn play_0x34_not_ignored() {
        // 0x34 已接入派发，不应再落到 Ignored。
        let pkt = play::ClientHeldItemChange { slot: 3 };
        let out = dispatch(ConnectionState::Play, 0x34, &payload_of(&pkt)).unwrap();
        assert!(
            !matches!(out, InboundPacket::Ignored { .. }),
            "0x34 不应被忽略，实际 {out:?}"
        );
    }

    #[test]
    fn close_container_roundtrip() {
        // CloseContainer 编码后解码应等于原包。
        let p = play::CloseContainer { window_id: 7 };
        let payload = payload_of(&p);
        let decoded = play::CloseContainer::decode(&mut ByteBuffer::new(payload.clone())).unwrap();
        assert_eq!(decoded, p);
    }

    #[test]
    fn client_held_item_change_roundtrip() {
        // ClientHeldItemChange 负 short 是合法线格式，必须往返。
        let p = play::ClientHeldItemChange { slot: -1 };
        let payload = payload_of(&p);
        let decoded =
            play::ClientHeldItemChange::decode(&mut ByteBuffer::new(payload.clone())).unwrap();
        assert_eq!(decoded, p);
    }

    #[test]
    fn held_item_change_clientbound_roundtrip() {
        // HeldItemChange（clientbound 回发确认）编码后解码应等于原包。
        let p = play::HeldItemChange { slot: 3 };
        let payload = payload_of(&p);
        let decoded = play::HeldItemChange::decode(&mut ByteBuffer::new(payload.clone())).unwrap();
        assert_eq!(decoded, p);
    }
}
