//! BossBar API（框架层，见 `.specs/implement-framework-capabilities/` R8）。
//!
//! 提供 [`BossBar`] 值类型与 [`BossBarManager`] `Resource`，维护 BossBar 的
//! 创建 / 更新 / 移除状态，并生成 T20 已实现的
//! [`crate::protocol::packets::play::BossBar`]（0x09）clientbound 包。
//!
//! **命名说明**：本模块的 [`BossBar`]（资源值类型，不含 `action`）与协议包
//! `protocol::packets::play::BossBar`（含 `action`）同名，协议包在方法签名中以
//! [`BossBarPacket`] 别名呈现。包生成方法的返回类型即协议包。
//!
//! 包的发送路径由 T28 系统接线接入；本模块只维护状态与包生成。

use std::collections::HashMap;

use uuid::Uuid;

use crate::protocol::packets::play::BossBar as BossBarPacket;
use crate::protocol::packets::play::BossBarAction;

/// BossBar 颜色（VarInt 编码）。
///
/// | 变体        | VarInt 值 |
/// |------------|----------|
/// | `Pink`     | 0        |
/// | `Blue`     | 1        |
/// | `Red`      | 2        |
/// | `Green`    | 3        |
/// | `Yellow`   | 4        |
/// | `Purple`   | 5        |
/// | `White`    | 6        |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BossBarColor {
    #[default]
    Pink,
    Blue,
    Red,
    Green,
    Yellow,
    Purple,
    White,
}

impl BossBarColor {
    /// 序列化为本协议 VarInt 值。
    pub fn as_i32(self) -> i32 {
        match self {
            BossBarColor::Pink => 0,
            BossBarColor::Blue => 1,
            BossBarColor::Red => 2,
            BossBarColor::Green => 3,
            BossBarColor::Yellow => 4,
            BossBarColor::Purple => 5,
            BossBarColor::White => 6,
        }
    }
}

/// BossBar 分节样式（VarInt 编码）。
///
/// | 变体           | VarInt 值 |
/// |---------------|----------|
/// | `Notched6`    | 0        |
/// | `Notched10`   | 1        |
/// | `Notched12`   | 2        |
/// | `Notched20`   | 3        |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BossBarDivision {
    #[default]
    Notched6,
    Notched10,
    Notched12,
    Notched20,
}

impl BossBarDivision {
    /// 序列化为本协议 VarInt 值。
    pub fn as_i32(self) -> i32 {
        match self {
            BossBarDivision::Notched6 => 0,
            BossBarDivision::Notched10 => 1,
            BossBarDivision::Notched12 => 2,
            BossBarDivision::Notched20 => 3,
        }
    }
}

/// BossBar 状态（创建 / 更新数据的值类型）。
#[derive(Debug, Clone, PartialEq)]
pub struct BossBar {
    /// BossBar 唯一标识（UUID）。
    pub uuid: Uuid,
    /// 标题（简化 String）。
    pub title: String,
    /// 血量比例（0.0 ~ 1.0）。
    pub health: f32,
    /// 颜色。
    pub color: BossBarColor,
    /// 分节。
    pub division: BossBarDivision,
    /// 标志位（Byte）。
    pub flags: u8,
}

/// BossBar 管理器（旧 ECS 方案 `Resource`）。
///
/// 由 [`crate::plugin::McServerPlugin`] 装配或在应用侧自行插入。
#[derive(Default)]
pub struct BossBarManager {
    /// BossBar 表（UUID → 状态）。
    bars: HashMap<Uuid, BossBar>,
}

impl BossBarManager {
    /// 添加一条 BossBar；uuid 重复返回 [`BossBarError::DuplicateUuid`]。
    pub fn add(&mut self, bar: BossBar) -> Result<(), BossBarError> {
        if self.bars.contains_key(&bar.uuid) {
            return Err(BossBarError::DuplicateUuid(bar.uuid));
        }
        self.bars.insert(bar.uuid, bar);
        Ok(())
    }

    /// 更新血量比例，返回是否存在。
    pub fn update_health(&mut self, uuid: Uuid, health: f32) -> bool {
        match self.bars.get_mut(&uuid) {
            Some(bar) => {
                bar.health = health;
                true
            }
            None => false,
        }
    }

    /// 更新标题，返回是否存在。
    pub fn update_title(&mut self, uuid: Uuid, title: String) -> bool {
        match self.bars.get_mut(&uuid) {
            Some(bar) => {
                bar.title = title;
                true
            }
            None => false,
        }
    }

    /// 更新颜色与分节，返回是否存在。
    pub fn update_style(
        &mut self,
        uuid: Uuid,
        color: BossBarColor,
        division: BossBarDivision,
    ) -> bool {
        match self.bars.get_mut(&uuid) {
            Some(bar) => {
                bar.color = color;
                bar.division = division;
                true
            }
            None => false,
        }
    }

    /// 移除一条 BossBar，返回是否存在。
    pub fn remove(&mut self, uuid: Uuid) -> bool {
        self.bars.remove(&uuid).is_some()
    }

    /// 查询一条 BossBar（只读）。
    pub fn get(&self, uuid: Uuid) -> Option<&BossBar> {
        self.bars.get(&uuid)
    }

    /// 当前 BossBar 数量。
    pub fn len(&self) -> usize {
        self.bars.len()
    }

    /// 是否没有任何 BossBar。
    pub fn is_empty(&self) -> bool {
        self.bars.is_empty()
    }

    /// 生成创建包（`BossBarAction::Add`）；不存在返回 `None`。
    pub fn add_packet(&self, uuid: Uuid) -> Option<BossBarPacket> {
        let bar = self.bars.get(&uuid)?;
        Some(BossBarPacket {
            uuid,
            action: BossBarAction::Add {
                title: bar.title.clone(),
                health: bar.health,
                color: bar.color.as_i32(),
                division: bar.division.as_i32(),
                flags: bar.flags,
            },
        })
    }

    /// 生成更新血量包（`BossBarAction::UpdateHealth`）；不存在返回 `None`。
    pub fn update_health_packet(&self, uuid: Uuid) -> Option<BossBarPacket> {
        let bar = self.bars.get(&uuid)?;
        Some(BossBarPacket {
            uuid,
            action: BossBarAction::UpdateHealth(bar.health),
        })
    }

    /// 生成更新标题包（`BossBarAction::UpdateTitle`）；不存在返回 `None`。
    pub fn update_title_packet(&self, uuid: Uuid) -> Option<BossBarPacket> {
        let bar = self.bars.get(&uuid)?;
        Some(BossBarPacket {
            uuid,
            action: BossBarAction::UpdateTitle(bar.title.clone()),
        })
    }

    /// 生成更新样式包（`BossBarAction::UpdateStyle`）；不存在返回 `None`。
    pub fn update_style_packet(&self, uuid: Uuid) -> Option<BossBarPacket> {
        let bar = self.bars.get(&uuid)?;
        Some(BossBarPacket {
            uuid,
            action: BossBarAction::UpdateStyle {
                color: bar.color.as_i32(),
                division: bar.division.as_i32(),
            },
        })
    }

    /// 生成更新标志位包（`BossBarAction::UpdateFlags`）；不存在返回 `None`。
    pub fn update_flags_packet(&self, uuid: Uuid) -> Option<BossBarPacket> {
        let bar = self.bars.get(&uuid)?;
        Some(BossBarPacket {
            uuid,
            action: BossBarAction::UpdateFlags(bar.flags),
        })
    }

    /// 生成移除包（`BossBarAction::Remove`）；不存在返回 `None`。
    pub fn remove_packet(&self, uuid: Uuid) -> Option<BossBarPacket> {
        if self.bars.contains_key(&uuid) {
            Some(BossBarPacket {
                uuid,
                action: BossBarAction::Remove,
            })
        } else {
            None
        }
    }
}

/// BossBar 操作错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BossBarError {
    /// uuid 已存在。
    DuplicateUuid(Uuid),
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn bar(uuid: Uuid) -> BossBar {
        BossBar {
            uuid,
            title: "凋灵".to_string(),
            health: 1.0,
            color: BossBarColor::Pink,
            division: BossBarDivision::Notched6,
            flags: 0x01,
        }
    }

    #[test]
    fn add_duplicate_and_remove() {
        let mut mgr = BossBarManager::default();
        let uuid = Uuid::new_v4();
        assert_eq!(mgr.add(bar(uuid)), Ok(()));
        assert_eq!(mgr.add(bar(uuid)), Err(BossBarError::DuplicateUuid(uuid)));
        assert_eq!(mgr.len(), 1);
        assert!(!mgr.is_empty());

        assert!(mgr.remove(uuid));
        assert!(!mgr.remove(uuid));
        assert!(mgr.is_empty());
    }

    #[test]
    fn update_methods_and_flags() {
        let mut mgr = BossBarManager::default();
        let uuid = Uuid::new_v4();
        mgr.add(bar(uuid)).unwrap();

        assert!(mgr.update_health(uuid, 0.5));
        assert!(mgr.update_title(uuid, "新标题".to_string()));
        assert!(mgr.update_style(uuid, BossBarColor::Green, BossBarDivision::Notched10));
        // 未登记 uuid 更新失败
        assert!(!mgr.update_health(Uuid::nil(), 0.5));

        let b = mgr.get(uuid).unwrap();
        assert_eq!(b.health, 0.5);
        assert_eq!(b.title, "新标题");
        assert_eq!(b.color, BossBarColor::Green);
        assert_eq!(b.division, BossBarDivision::Notched10);
        assert_eq!(b.flags, 0x01);
    }

    #[test]
    fn add_packet_fields() {
        let mut mgr = BossBarManager::default();
        let uuid = Uuid::new_v4();
        mgr.add(bar(uuid)).unwrap();

        let pkt = mgr.add_packet(uuid).expect("add_packet 应在");
        assert_eq!(pkt.uuid, uuid);
        match pkt.action {
            BossBarAction::Add {
                title,
                health,
                color,
                division,
                flags,
            } => {
                assert_eq!(title, "凋灵");
                assert_eq!(health, 1.0);
                assert_eq!(color, 0);
                assert_eq!(division, 0);
                assert_eq!(flags, 0x01);
            }
            _ => panic!("期望 Add 动作"),
        }
        assert!(mgr.add_packet(Uuid::nil()).is_none());
    }

    #[test]
    fn update_and_remove_packets() {
        let mut mgr = BossBarManager::default();
        let uuid = Uuid::new_v4();
        mgr.add(bar(uuid)).unwrap();
        mgr.update_health(uuid, 0.25);

        match mgr.update_health_packet(uuid).unwrap().action {
            BossBarAction::UpdateHealth(h) => assert_eq!(h, 0.25),
            _ => panic!("期望 UpdateHealth 动作"),
        }
        match mgr.update_title_packet(uuid).unwrap().action {
            BossBarAction::UpdateTitle(t) => assert_eq!(t, "凋灵"),
            _ => panic!("期望 UpdateTitle 动作"),
        }
        match mgr.update_style_packet(uuid).unwrap().action {
            BossBarAction::UpdateStyle { color, division } => {
                assert_eq!(color, 0);
                assert_eq!(division, 0);
            }
            _ => panic!("期望 UpdateStyle 动作"),
        }
        match mgr.update_flags_packet(uuid).unwrap().action {
            BossBarAction::UpdateFlags(f) => assert_eq!(f, 0x01),
            _ => panic!("期望 UpdateFlags 动作"),
        }
        match mgr.remove_packet(uuid).unwrap().action {
            BossBarAction::Remove => {}
            _ => panic!("期望 Remove 动作"),
        }
        // 未登记 uuid 不生成任何更新/移除包
        assert!(mgr.update_health_packet(Uuid::nil()).is_none());
        assert!(mgr.remove_packet(Uuid::nil()).is_none());
    }
}
