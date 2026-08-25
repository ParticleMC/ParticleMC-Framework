// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 计分板 API（框架层，见 `.specs/implement-framework-capabilities/` R8）。
//!
//! 提供 [`Objective`]（目标）、[`Team`]（队伍）、[`ScoreEntry`]（分数条目）三个
//! 值类型，以及 [`Scoreboard`] `Resource` 作为计分板状态容器。状态经 T20 已实现的
//! clientbound 包同步到客户端：
//!
//! - [`ScoreboardObjective`]（0x68）：目标创建 / 更新 / 移除
//! - [`Teams`]（0x6b）：队伍创建（含成员）
//! - [`UpdateScore`]（0x6c）：分数写入
//! - [`DisplayScoreboard`]（0x60）：目标挂到显示位置（列表 / 侧栏 / 下方）
//!
//! 本模块只维护框架状态并生成包，包的发送路径由 T28 系统接线接入。

use std::collections::HashMap;

use crate::protocol::packets::play::{DisplayScoreboard, ScoreboardObjective, Teams, UpdateScore};

/// 目标未挂载任何显示位置时的缺省 `display_slot`（-1 表示「不显示」）。
const DEFAULT_DISPLAY_SLOT: i8 = -1;

/// 计分板目标。
///
/// 对应框架的 `Objective`（见 `.specs/implement-framework-capabilities/` R8）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Objective {
    /// 目标名（协议同步键）。
    pub name: String,
    /// 显示名（简化 String）。
    pub display_name: String,
    /// 显示类型（0=整数，1=心形）。
    pub objective_type: i32,
    /// 当前挂载的显示位置（0=列表，1=侧栏，2=下方；-1=未挂载）。
    pub display_slot: i8,
}

/// 计分板队伍。
///
/// 对应框架的 `Team`（见 `.specs/implement-framework-capabilities/` R8）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Team {
    /// 队伍名（协议同步键）。
    pub name: String,
    /// 显示名（简化 String）。
    pub display_name: String,
    /// 名称前缀（简化 String）。
    pub prefix: String,
    /// 名称后缀（简化 String）。
    pub suffix: String,
    /// 队伍颜色（VarInt）。
    pub color: i32,
    /// 成员名列表。
    pub members: Vec<String>,
}

/// 计分板分数条目（实体 + 目标 → 分数）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoreEntry {
    /// 实体名。
    pub entity_name: String,
    /// 所属目标名。
    pub objective_name: String,
    /// 分数值。
    pub value: i32,
}

/// 计分板状态容器（旧 ECS 方案 `Resource`）。
///
/// 由 [`crate::plugin::McServerPlugin`] 装配或在应用侧自行插入。操作失败返回
/// [`ScoreboardError`]；包生成方法只读取当前状态。
#[derive(Default)]
pub struct Scoreboard {
    /// 目标表（名称 → 目标）。
    objectives: HashMap<String, Objective>,
    /// 队伍表（名称 → 队伍）。
    teams: HashMap<String, Team>,
    /// 分数条目（可重复实体名，按目标区分）。
    scores: Vec<ScoreEntry>,
    /// 显示位置 → 目标名。
    display: HashMap<i8, String>,
}

impl Scoreboard {
    /// 创建一个目标；名称已存在返回 [`ScoreboardError::ObjectiveExists`]。
    ///
    /// 新目标未挂载显示位置（`display_slot = -1`），可用
    /// [`display_objective`](Self::display_objective) 挂载。
    pub fn create_objective(
        &mut self,
        name: &str,
        display_name: &str,
        objective_type: i32,
    ) -> Result<(), ScoreboardError> {
        if self.objectives.contains_key(name) {
            return Err(ScoreboardError::ObjectiveExists);
        }
        self.objectives.insert(
            name.to_string(),
            Objective {
                name: name.to_string(),
                display_name: display_name.to_string(),
                objective_type,
                display_slot: DEFAULT_DISPLAY_SLOT,
            },
        );
        Ok(())
    }

    /// 移除一个目标；存在则同时清理其显示位置与分数条目，返回是否存在。
    pub fn remove_objective(&mut self, name: &str) -> bool {
        if self.objectives.remove(name).is_none() {
            return false;
        }
        self.scores.retain(|s| s.objective_name != name);
        self.display.retain(|_slot, obj| obj != name);
        true
    }

    /// 把目标挂到指定显示位置（覆盖旧映射，不要求目标已存在）。
    ///
    /// 位置取值为 Minecraft 计分板槽位（0=列表，1=侧栏，2=下方）。
    pub fn display_objective(&mut self, slot: i8, objective_name: String) {
        if let Some(o) = self.objectives.get_mut(objective_name.as_str()) {
            o.display_slot = slot;
        }
        self.display.insert(slot, objective_name);
    }

    /// 创建一个队伍；队伍名已存在返回 [`ScoreboardError::TeamExists`]。
    pub fn create_team(&mut self, team: Team) -> Result<(), ScoreboardError> {
        if self.teams.contains_key(&team.name) {
            return Err(ScoreboardError::TeamExists);
        }
        self.teams.insert(team.name.clone(), team);
        Ok(())
    }

    /// 移除一个队伍，返回是否存在。
    pub fn remove_team(&mut self, name: &str) -> bool {
        self.teams.remove(name).is_some()
    }

    /// 写入（或更新）一条分数；目标不存在返回 [`ScoreboardError::NotFound`]。
    pub fn set_score(
        &mut self,
        entity: &str,
        objective: &str,
        value: i32,
    ) -> Result<(), ScoreboardError> {
        if !self.objectives.contains_key(objective) {
            return Err(ScoreboardError::NotFound);
        }
        if let Some(entry) = self
            .scores
            .iter_mut()
            .find(|e| e.entity_name == entity && e.objective_name == objective)
        {
            entry.value = value;
        } else {
            self.scores.push(ScoreEntry {
                entity_name: entity.to_string(),
                objective_name: objective.to_string(),
                value,
            });
        }
        Ok(())
    }

    /// 移除指定实体在某目标下的分数（不存在则无操作）。
    pub fn reset_score(&mut self, entity: &str, objective: &str) {
        self.scores
            .retain(|e| !(e.entity_name == entity && e.objective_name == objective));
    }

    /// 生成目标创建包（action 0）；目标不存在返回空列表。
    pub fn objective_packets(&self, name: &str) -> Vec<ScoreboardObjective> {
        let o = match self.objectives.get(name) {
            Some(o) => o,
            None => return Vec::new(),
        };
        vec![ScoreboardObjective {
            objective_name: o.name.clone(),
            action: 0,
            display_name: o.display_name.clone(),
            objective_type: o.objective_type,
        }]
    }

    /// 生成队伍创建包（action 0，含成员）；队伍不存在返回空列表。
    pub fn team_packets(&self, name: &str) -> Vec<Teams> {
        let t = match self.teams.get(name) {
            Some(t) => t,
            None => return Vec::new(),
        };
        vec![Teams {
            team_name: t.name.clone(),
            action: 0,
            display_name: t.display_name.clone(),
            prefix: t.prefix.clone(),
            suffix: t.suffix.clone(),
            color: t.color,
            members: t.members.clone(),
        }]
    }

    /// 生成全部分数的更新包（action 0，携带数值）。
    pub fn score_packets(&self) -> Vec<UpdateScore> {
        self.scores
            .iter()
            .map(|s| UpdateScore {
                entity_name: s.entity_name.clone(),
                action: 0,
                objective_name: s.objective_name.clone(),
                value: Some(s.value),
            })
            .collect()
    }

    /// 生成全部显示位置映射的包。
    pub fn display_packets(&self) -> Vec<DisplayScoreboard> {
        self.display
            .iter()
            .map(|(slot, name)| DisplayScoreboard {
                position: *slot,
                objective_name: name.clone(),
            })
            .collect()
    }
}

/// 计分板操作错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScoreboardError {
    /// 目标名已存在。
    ObjectiveExists,
    /// 队伍名已存在。
    TeamExists,
    /// 引用的目标不存在。
    NotFound,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn objective_crud_and_packets() {
        let mut sb = Scoreboard::default();
        assert_eq!(sb.create_objective("kill", "击杀", 0), Ok(()));
        // 重复创建报错
        assert_eq!(
            sb.create_objective("kill", "击杀", 0),
            Err(ScoreboardError::ObjectiveExists)
        );

        let pkts = sb.objective_packets("kill");
        assert_eq!(pkts.len(), 1);
        let pkt = &pkts[0];
        assert_eq!(pkt.objective_name, "kill");
        assert_eq!(pkt.action, 0);
        assert_eq!(pkt.display_name, "击杀");
        assert_eq!(pkt.objective_type, 0);

        // 不存在的目标返回空包
        assert!(sb.objective_packets("missing").is_empty());

        assert!(sb.remove_objective("kill"));
        assert!(!sb.remove_objective("kill"));
        assert!(sb.objective_packets("kill").is_empty());
    }

    #[test]
    fn team_crud_and_packets() {
        let mut sb = Scoreboard::default();
        let team = Team {
            name: "red".to_string(),
            display_name: "红队".to_string(),
            prefix: "[R]".to_string(),
            suffix: "".to_string(),
            color: 1,
            members: vec!["Alice".to_string(), "Bob".to_string()],
        };
        assert_eq!(sb.create_team(team.clone()), Ok(()));
        assert_eq!(sb.create_team(team), Err(ScoreboardError::TeamExists));

        let pkts = sb.team_packets("red");
        assert_eq!(pkts.len(), 1);
        let pkt = &pkts[0];
        assert_eq!(pkt.team_name, "red");
        assert_eq!(pkt.action, 0);
        assert_eq!(pkt.display_name, "红队");
        assert_eq!(pkt.prefix, "[R]");
        assert_eq!(pkt.suffix, "");
        assert_eq!(pkt.color, 1);
        assert_eq!(pkt.members, vec!["Alice".to_string(), "Bob".to_string()]);

        assert!(sb.remove_team("red"));
        assert!(!sb.remove_team("red"));
        assert!(sb.team_packets("red").is_empty());
    }

    #[test]
    fn score_set_reset_and_packets() {
        let mut sb = Scoreboard::default();
        // 目标不存在时报错
        assert_eq!(
            sb.set_score("p1", "kill", 5),
            Err(ScoreboardError::NotFound)
        );

        sb.create_objective("kill", "击杀", 0).unwrap();
        sb.set_score("p1", "kill", 5).unwrap();
        sb.set_score("p2", "kill", 3).unwrap();
        // 同实体同目标更新
        sb.set_score("p1", "kill", 7).unwrap();

        let pkts = sb.score_packets();
        assert_eq!(pkts.len(), 2);
        let p1 = pkts
            .iter()
            .find(|p| p.entity_name == "p1")
            .expect("p1 分数包应在");
        assert_eq!(p1.action, 0);
        assert_eq!(p1.objective_name, "kill");
        assert_eq!(p1.value, Some(7));
        let p2 = pkts
            .iter()
            .find(|p| p.entity_name == "p2")
            .expect("p2 分数包应在");
        assert_eq!(p2.value, Some(3));

        sb.reset_score("p1", "kill");
        assert_eq!(sb.score_packets().len(), 1);
        // 目标移除时清理其分数
        sb.remove_objective("kill");
        assert!(sb.score_packets().is_empty());
    }

    #[test]
    fn display_mapping_and_packets() {
        let mut sb = Scoreboard::default();
        sb.create_objective("side", "侧栏", 0).unwrap();
        sb.display_objective(1, "side".to_string());

        let pkts = sb.display_packets();
        assert_eq!(pkts.len(), 1);
        assert_eq!(pkts[0].position, 1);
        assert_eq!(pkts[0].objective_name, "side");

        // 移除目标会清理其显示映射
        sb.remove_objective("side");
        assert!(sb.display_packets().is_empty());
    }
}
