//! 实体子系统（变更标识符：`complete-missing-subsystems`）。
//!
//! 本模块补齐此前缺失的实体域代码组织。当前交付：
//! - [`metadata`]：生物 metadata 全量类型（[`EntityMetaType`] 注册表 + 类型化
//!   核心 struct + 与 [`EntityMetadataMap`](crate::component::EntityMetadataMap)
//!   的互转）。
//! - [`ai`]：实体 AI 子系统（goal / target 选择器、内置目标行为与
//!   [`EntityAIGroup`]）。
//! - [`pathfinding`]：实体寻路子系统（A* 搜索、路径生成器与路径跟随器）。
//!
//! 后续子系统（实体 id 分配、出生逻辑等）将按 spec R3/R4 在此目录增量展开。

pub mod ai;
pub mod metadata;
pub mod pathfinding;

pub use ai::{
    ClosestEntityTarget, CombinedAttackGoal, DoNothingGoal, EntityAIGroup, EntityFilter,
    FollowTargetGoal, Goal, GoalContext, GoalSelector, LastEntityDamagerTarget, MeleeAttackGoal,
    RandomLookAroundGoal, RandomStrollGoal, RangedAttackGoal, Target, TargetSelector,
};
pub use pathfinding::{
    FlyingNodeFollower, FlyingNodeGenerator, GroundNodeFollower, GroundNodeGenerator, Navigator,
    NoPhysicsNodeFollower, NodeFollower, PPath, PathGenerator, PreciseGroundNodeGenerator,
    WaterNodeFollower, WaterNodeGenerator, a_star,
};
