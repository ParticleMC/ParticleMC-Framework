//! 进度（Advancement）API（框架层，见 `.specs/implement-framework-capabilities/` R8）。
//!
//! 提供 [`Advancement`] 值类型与 [`AdvancementManager`] `Resource`，维护进度树的
//! 注册 / 移除状态，并生成 T20 已实现的 [`Advancements`]（0x80）clientbound 包
//! （`clear=false` + 全部进度三元组 + 待移除 id 列表）。
//!
//! 包的发送路径由 T28 系统接线接入；本模块只维护状态与包生成。

use crate::protocol::packets::play::Advancements;

/// 单个进度节点。
///
/// `display_title` / `display_description` 为显示数据；T20 的 `Advancements` 包为
/// **简化**格式（省略 display_data），故包同步时这两个字段不参与编码。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advancement {
    /// 进度 id（唯一，注册键）。
    pub id: String,
    /// 父进度 id；`None` 表示根进度。
    pub parent: Option<String>,
    /// 完成条件（criteria）列表。
    pub criteria: Vec<String>,
    /// 显示标题（简化 String；可选）。
    pub display_title: Option<String>,
    /// 显示描述（简化 String；可选）。
    pub display_description: Option<String>,
}

/// 进度管理器（旧 ECS 方案 `Resource`）。
///
/// 由 [`crate::plugin::McServerPlugin`] 装配或在应用侧自行插入。
#[derive(Default)]
pub struct AdvancementManager {
    /// 已注册进度（保持注册顺序）。
    advancements: Vec<Advancement>,
    /// 待移除进度 id（随包下发后由应用侧决定是否清理）。
    removed: Vec<String>,
}

impl AdvancementManager {
    /// 注册一个进度；id 重复返回 [`AdvancementError::DuplicateId`]。
    pub fn register(&mut self, a: Advancement) -> Result<(), AdvancementError> {
        if self.advancements.iter().any(|x| x.id == a.id) {
            return Err(AdvancementError::DuplicateId(a.id));
        }
        self.advancements.push(a);
        Ok(())
    }

    /// 移除一个进度并记入待移除列表，返回是否存在。
    ///
    /// 进度 id 未注册时返回 `false` 且不产生移除记录。
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.advancements.len();
        self.advancements.retain(|a| a.id != id);
        if self.advancements.len() == before {
            return false;
        }
        self.removed.push(id.to_string());
        true
    }

    /// 生成 `Advancements` 同步包：`clear=false` + 全部已注册进度 + 待移除 id 列表。
    pub fn packet(&self) -> Advancements {
        Advancements {
            clear: false,
            advancements: self
                .advancements
                .iter()
                .map(|a| (a.id.clone(), a.parent.clone(), a.criteria.clone()))
                .collect(),
            removed: self.removed.clone(),
        }
    }
}

/// 进度操作错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdvancementError {
    /// 进度 id 已注册。
    DuplicateId(String),
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn advancement(id: &str, parent: Option<&str>) -> Advancement {
        Advancement {
            id: id.to_string(),
            parent: parent.map(|p| p.to_string()),
            criteria: vec!["done".to_string()],
            display_title: None,
            display_description: None,
        }
    }

    #[test]
    fn register_duplicate_and_remove() {
        let mut mgr = AdvancementManager::default();
        assert_eq!(mgr.register(advancement("a", None)), Ok(()));
        assert_eq!(
            mgr.register(advancement("a", None)),
            Err(AdvancementError::DuplicateId("a".to_string()))
        );
        assert!(mgr.remove("a"));
        assert!(!mgr.remove("a"));
        assert!(!mgr.remove("missing"));
    }

    #[test]
    fn packet_fields() {
        let mut mgr = AdvancementManager::default();
        mgr.register(advancement("root", None)).unwrap();
        mgr.register(advancement("child", Some("root"))).unwrap();
        mgr.remove("child");

        let pkt = mgr.packet();
        assert!(!pkt.clear);
        // 剩余进度只有 root
        assert_eq!(pkt.advancements.len(), 1);
        assert_eq!(pkt.advancements[0].0, "root");
        assert_eq!(pkt.advancements[0].1, None);
        assert_eq!(pkt.advancements[0].2, vec!["done".to_string()]);
        // 待移除列表包含 child
        assert_eq!(pkt.removed, vec!["child".to_string()]);
    }

    #[test]
    fn empty_packet() {
        let mgr = AdvancementManager::default();
        let pkt = mgr.packet();
        assert!(!pkt.clear);
        assert!(pkt.advancements.is_empty());
        assert!(pkt.removed.is_empty());
    }
}
