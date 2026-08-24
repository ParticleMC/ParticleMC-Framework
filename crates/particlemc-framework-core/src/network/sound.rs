// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 声音播放（T13，对应 spec R13）。
//!
//! 简化值类型 [`Sound`]（id / 分类）经 [`play_sound`] 转换为协议
//! `SoundEffect`(0x73) 包并入队。线格式中声音 id = 注册表 id + 1（0 保留给
//! 自定义声音），坐标以 1/8 方块定点整数承载，种子恒 0（v1 简化）。
//!
//! 变更标识符：`complete-missing-subsystems`（T13）。

use crate::network::client::{ClientNetworks, Priority, enqueue_packet};
use crate::protocol::packets::{encode_clientbound, play};

/// 简化声音值类型。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sound {
    /// 声音注册表 id（`minecraft:sound_event` 序位，线格式值 = id + 1）。
    pub id: u32,
    /// 声音分类（0=大师、1=音乐、2=记录、3=天气、4=方块、5=敌对、6=中立、
    /// 7=玩家、8=环境、9=语音）。
    pub category: i32,
}

/// 把 f64 世界坐标换算为 1/8 方块定点整数（四舍五入后收敛到 `i32` 范围）。
fn fixed_point(value: f64) -> i32 {
    (value * 8.0)
        .round()
        .clamp(i32::MIN as f64, i32::MAX as f64) as i32
}

/// 向单个连接播放一个声音效果（`SoundEffect`(0x73) 包，普通优先级入队）。
///
/// 声音出现在世界坐标 `position`，`volume` / `pitch` 透传。
/// 连接不存在时静默丢弃（不 panic）。
pub fn play_sound(
    clients: &mut ClientNetworks,
    conn_id: u32,
    sound: &Sound,
    position: [f64; 3],
    volume: f32,
    pitch: f32,
) {
    let [x, y, z] = position;
    let packet = play::SoundEffect {
        sound_id: i32::try_from(sound.id)
            .unwrap_or(i32::MAX)
            .saturating_add(1),
        sound_category: sound.category,
        x: fixed_point(x),
        y: fixed_point(y),
        z: fixed_point(z),
        volume,
        pitch,
        seed: 0,
    };
    enqueue_packet(
        clients,
        conn_id,
        encode_clientbound(&packet),
        Priority::Normal,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::byte_buf::ByteBuffer;
    use crate::protocol::packet::Packet;

    fn decode_payload(bytes: &[u8]) -> (i32, play::SoundEffect) {
        let mut buf = ByteBuffer::new(bytes.to_vec());
        let id = buf.get_varint().unwrap();
        let sound = play::SoundEffect::decode(&mut buf).unwrap();
        (id, sound)
    }

    #[test]
    fn play_sound_enqueues_sound_effect_packet() {
        let mut clients = ClientNetworks::default();
        clients.insert(1);
        let sound = Sound { id: 3, category: 4 };
        play_sound(&mut clients, 1, &sound, [1.0, 2.0, 3.0], 0.8, 1.2);

        assert_eq!(clients.clients[&1].normal_queue.len(), 1);
        let bytes = &clients.clients[&1].normal_queue[0];
        let (packet_id, packet) = decode_payload(bytes);
        assert_eq!(packet_id, 0x73, "SoundEffect 包 id 应为 0x73");
        // 线格式声音 id = 注册表 id + 1。
        assert_eq!(packet.sound_id, 4);
        assert_eq!(packet.sound_category, 4);
        // 1/8 方块定点：1.0 → 8，2.0 → 16，3.0 → 24。
        assert_eq!((packet.x, packet.y, packet.z), (8, 16, 24));
        assert_eq!(packet.volume, 0.8);
        assert_eq!(packet.pitch, 1.2);
        assert_eq!(packet.seed, 0);
    }

    #[test]
    fn fixed_point_rounds_nearest() {
        assert_eq!(fixed_point(0.0), 0);
        assert_eq!(fixed_point(0.5 / 8.0), 1);
        assert_eq!(fixed_point(-0.5 / 8.0), -1);
        // 负方向取整到更近的整数定点。
        assert_eq!(fixed_point(-0.25), -2);
    }

    #[test]
    fn play_sound_to_unknown_connection_is_noop() {
        let mut clients = ClientNetworks::default();
        let sound = Sound { id: 1, category: 0 };
        play_sound(&mut clients, 404, &sound, [0.0, 0.0, 0.0], 1.0, 1.0);
        assert!(clients.clients.is_empty());
    }
}
