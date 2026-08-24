// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 粒子发送（T13，对应 spec R13）。
//!
//! 简化值类型 [`Particle`]（id / 数量 / 速度）经 [`send_particle`] 转换为
//! 协议 `Particle`(0x2e) 包并入队到 [`ClientNetworks`] 的普通队列。
//! 线格式复用 `play::Particle`（偏移量恒 0、非远距离渲染）。
//!
//! 变更标识符：`complete-missing-subsystems`（T13）。

use crate::network::client::{ClientNetworks, Priority, enqueue_packet};
use crate::protocol::packets::{encode_clientbound, play};

/// 简化粒子值类型。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Particle {
    /// 粒子类型 id（`minecraft:particle_type` 注册表序位）。
    pub id: u32,
    /// 粒子数量。
    pub count: i32,
    /// 最大速度（每 tick 位移，f32）。
    pub speed: f32,
}

/// 向单个连接发送一个粒子效果（`Particle`(0x2e) 包，普通优先级入队）。
///
/// 粒子出现在世界坐标 `position`，使用均匀球状扩散速度场（偏移量恒 0）。
/// 连接不存在时静默丢弃（不 panic）。
pub fn send_particle(
    clients: &mut ClientNetworks,
    conn_id: u32,
    particle: &Particle,
    position: [f64; 3],
) {
    let [x, y, z] = position;
    let packet = play::Particle {
        particle_id: i32::try_from(particle.id).unwrap_or(i32::MAX),
        long_distance: false,
        x,
        y,
        z,
        offset_x: 0.0,
        offset_y: 0.0,
        offset_z: 0.0,
        max_speed: particle.speed,
        particle_count: particle.count,
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

    fn decode_payload(bytes: &[u8]) -> (i32, play::Particle) {
        let mut buf = ByteBuffer::new(bytes.to_vec());
        let id = buf.get_varint().unwrap();
        let particle = play::Particle::decode(&mut buf).unwrap();
        (id, particle)
    }

    #[test]
    fn send_particle_enqueues_particle_packet() {
        let mut clients = ClientNetworks::default();
        clients.insert(1);
        let particle = Particle {
            id: 2,
            count: 10,
            speed: 0.1,
        };
        send_particle(&mut clients, 1, &particle, [1.5, 64.0, -3.25]);

        assert_eq!(clients.clients[&1].normal_queue.len(), 1);
        let bytes = &clients.clients[&1].normal_queue[0];
        let (packet_id, packet) = decode_payload(bytes);
        assert_eq!(packet_id, 0x2e, "Particle 包 id 应为 0x2e");
        assert_eq!(packet.particle_id, 2);
        assert_eq!(packet.x, 1.5);
        assert_eq!(packet.y, 64.0);
        assert_eq!(packet.z, -3.25);
        assert_eq!(packet.particle_count, 10);
        assert_eq!(packet.max_speed, 0.1);
        assert!(!packet.long_distance);
    }

    #[test]
    fn send_particle_to_unknown_connection_is_noop() {
        let mut clients = ClientNetworks::default();
        let particle = Particle {
            id: 1,
            count: 5,
            speed: 0.0,
        };
        send_particle(&mut clients, 404, &particle, [0.0, 0.0, 0.0]);
        assert!(clients.clients.is_empty());
    }
}
