//! 实体寻路子系统：A* 搜索、路径生成器与路径跟随器（T6）。
//!
//! 语义对齐 Minestom Java 的 `entity/pathfinding` 包，但以简化的纯函数
//! 结构实现，不依赖世界实例（`is_solid` 判定由调用方以闭包注入）：
//!
//! - [`a_star`]：默认 8 邻地面 A*（含对角代价，八向启发式），`max_steps`
//!   防止搜索爆炸；不可达返回 `None`。
//! - [`PathGenerator`]：v1 共享 [`a_star`] 内核，生成器仅定制邻域与启发式
//!   —— [`GroundNodeGenerator`]（4 邻曼哈顿）、[`PreciseGroundNodeGenerator`]
//!   （8 邻 + 对角代价）、[`FlyingNodeGenerator`]（26 邻 3D 欧氏）、
//!   [`WaterNodeGenerator`]（4 邻水面）。
//! - [`Navigator`]：沿已寻路径推进的轻量状态机（`set_target` → `tick`）。
//! - [`NodeFollower`]：由当前位置与目标点计算下一 tick 速度向量。
//!
//! 简化说明（T6 报告注明）：PNode 提供公开结构供外部检视，A* 内部以索引
//! 表示的父链接还原路径；地面生成器 y 恒定（贴地简化，不含重力吸附）。
//!
//! 变更标识符：`complete-missing-subsystems`。

use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::f64::consts::SQRT_2;

/// A* 搜索节点（公开结构，供外部检视路径中间节点）。
pub struct PNode {
    /// 方块 X 坐标。
    pub x: i32,
    /// 方块 Y 坐标。
    pub y: i32,
    /// 方块 Z 坐标。
    pub z: i32,
    /// 已付代价（g 值）。
    pub g: f64,
    /// 启发估计（h 值）。
    pub h: f64,
    /// 父节点（路径还原链）。
    pub parent: Option<Box<PNode>>,
}

/// 已寻得路径：方块坐标列表，从起点到终点（含两端）。
#[derive(Clone, Debug, PartialEq)]
pub struct PPath {
    /// 路径节点列表。
    pub nodes: Vec<(i32, i32, i32)>,
}

impl PPath {
    /// 路径是否为空。
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// 路径节点数。
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
}

/// 启发式类型。
#[derive(Clone, Copy)]
enum Heuristic {
    /// 曼哈顿距离。
    Manhattan,
    /// 八向距离（对角代价 √2）。
    Octile,
    /// 欧氏距离。
    Euclidean,
}

/// 对角移动代价（√3 用于 3D 对角）。
const SQRT_3: f64 = 1.732_050_807_568_877_2;

/// 默认搜索步数上限（防爆）。
const MAX_STEPS: usize = 4096;

/// 4 邻域（正交地面移动）。
const NEIGHBORS_4: [(i32, i32, i32); 4] = [(1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1)];

/// 8 邻域（正交 + 对角地面移动）。
const NEIGHBORS_8: [(i32, i32, i32); 8] = [
    (1, 0, 0),
    (-1, 0, 0),
    (0, 0, 1),
    (0, 0, -1),
    (1, 0, 1),
    (1, 0, -1),
    (-1, 0, 1),
    (-1, 0, -1),
];

/// 26 邻域（3D 全方向飞行）。
const NEIGHBORS_26: [(i32, i32, i32); 26] = [
    (1, 0, 0),
    (-1, 0, 0),
    (0, 1, 0),
    (0, -1, 0),
    (0, 0, 1),
    (0, 0, -1),
    (1, 1, 0),
    (1, -1, 0),
    (-1, 1, 0),
    (-1, -1, 0),
    (1, 0, 1),
    (1, 0, -1),
    (-1, 0, 1),
    (-1, 0, -1),
    (0, 1, 1),
    (0, 1, -1),
    (0, -1, 1),
    (0, -1, -1),
    (1, 1, 1),
    (1, 1, -1),
    (1, -1, 1),
    (1, -1, -1),
    (-1, 1, 1),
    (-1, 1, -1),
    (-1, -1, 1),
    (-1, -1, -1),
];

/// A* 寻路（默认 8 邻地面，八向启发式）。
///
/// - `start` / `goal`：起终点方块坐标。
/// - `is_solid(x, y, z)`：指定方块是否实心（实心不可通行）。
/// - `max_steps`：防爆上限，超过则返回 `None`。
/// - 起点或终点实心、开集耗尽（不可达）均返回 `None`。
pub fn a_star(
    start: (i32, i32, i32),
    goal: (i32, i32, i32),
    is_solid: impl Fn(i32, i32, i32) -> bool,
    max_steps: usize,
) -> Option<PPath> {
    a_star_with(
        start,
        goal,
        &is_solid,
        max_steps,
        &NEIGHBORS_8,
        Heuristic::Octile,
    )
}

/// 带邻域与启发式配置的 A* 内核（供各生成器复用）。
fn a_star_with(
    start: (i32, i32, i32),
    goal: (i32, i32, i32),
    is_solid: &dyn Fn(i32, i32, i32) -> bool,
    max_steps: usize,
    neighbors: &[(i32, i32, i32)],
    heuristic: Heuristic,
) -> Option<PPath> {
    if is_solid(start.0, start.1, start.2) || is_solid(goal.0, goal.1, goal.2) {
        return None;
    }
    let mut open: BinaryHeap<Reverse<OpenItem>> = BinaryHeap::new();
    let mut g_score: HashMap<(i32, i32, i32), f64> = HashMap::new();
    let mut parent: HashMap<(i32, i32, i32), (i32, i32, i32)> = HashMap::new();
    let mut closed: HashSet<(i32, i32, i32)> = HashSet::new();

    let start_h = estimate(start, goal, heuristic);
    g_score.insert(start, 0.0);
    open.push(Reverse(OpenItem {
        f: start_h,
        tie: 0,
        pos: start,
    }));

    let mut tie = 1u64;
    let mut steps = 0usize;
    while let Some(Reverse(item)) = open.pop() {
        if steps >= max_steps {
            return None;
        }
        steps += 1;
        if item.pos == goal {
            return Some(reconstruct_path(start, goal, &parent));
        }
        if closed.contains(&item.pos) {
            continue;
        }
        closed.insert(item.pos);
        let g = g_score.get(&item.pos).copied().unwrap_or(f64::INFINITY);
        for (dx, dy, dz) in neighbors {
            let next = (item.pos.0 + dx, item.pos.1 + dy, item.pos.2 + dz);
            if is_solid(next.0, next.1, next.2) || closed.contains(&next) {
                continue;
            }
            let tentative = g + step_cost(*dx, *dy, *dz);
            let existing = g_score.get(&next).copied().unwrap_or(f64::INFINITY);
            if tentative < existing {
                g_score.insert(next, tentative);
                parent.insert(next, item.pos);
                tie += 1;
                open.push(Reverse(OpenItem {
                    f: tentative + estimate(next, goal, heuristic),
                    tie,
                    pos: next,
                }));
            }
        }
    }
    None
}

/// 依据邻域增量计算单步代价（对角 √2 / √3，正交 1）。
fn step_cost(dx: i32, dy: i32, dz: i32) -> f64 {
    if dx != 0 && dy != 0 && dz != 0 {
        SQRT_3
    } else if (dx != 0 && dz != 0) || (dx != 0 && dy != 0) || (dy != 0 && dz != 0) {
        SQRT_2
    } else {
        1.0
    }
}

/// 计算启发估计。
fn estimate(a: (i32, i32, i32), b: (i32, i32, i32), kind: Heuristic) -> f64 {
    let dx = f64::from((a.0 - b.0).abs());
    let dy = f64::from((a.1 - b.1).abs());
    let dz = f64::from((a.2 - b.2).abs());
    match kind {
        Heuristic::Manhattan => dx + dy + dz,
        Heuristic::Octile => {
            let (m, n) = (dx.max(dz), dx.min(dz));
            m + (SQRT_2 - 1.0) * n
        }
        Heuristic::Euclidean => (dx * dx + dy * dy + dz * dz).sqrt(),
    }
}

/// 由父链接表还原起点到终点的节点列表。
fn reconstruct_path(
    start: (i32, i32, i32),
    goal: (i32, i32, i32),
    parent: &HashMap<(i32, i32, i32), (i32, i32, i32)>,
) -> PPath {
    let mut nodes: Vec<(i32, i32, i32)> = Vec::new();
    let mut current = goal;
    loop {
        nodes.push(current);
        if current == start {
            break;
        }
        let Some(&prev) = parent.get(&current) else {
            break;
        };
        current = prev;
    }
    nodes.reverse();
    PPath { nodes }
}

/// 开放列表条目（`Reverse` 使二叉堆成为最小堆，`tie` 保证全序确定性）。
struct OpenItem {
    f: f64,
    tie: u64,
    pos: (i32, i32, i32),
}

impl PartialEq for OpenItem {
    fn eq(&self, other: &Self) -> bool {
        self.f == other.f && self.tie == other.tie
    }
}

impl Eq for OpenItem {}

impl Ord for OpenItem {
    fn cmp(&self, other: &Self) -> Ordering {
        self.f
            .partial_cmp(&other.f)
            .unwrap_or(Ordering::Equal)
            .then_with(|| self.tie.cmp(&other.tie))
    }
}

impl PartialOrd for OpenItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// 路径生成器：按自身邻域 / 启发式策略在方块世界中生成路径。
pub trait PathGenerator: Send + Sync {
    /// 生成从 `start` 到 `goal` 的路径；不可达返回 `None`。
    fn generate(
        &self,
        start: (i32, i32, i32),
        goal: (i32, i32, i32),
        is_solid: &dyn Fn(i32, i32, i32) -> bool,
    ) -> Option<PPath>;
}

/// 地面生成器：4 邻正交移动，y 恒定，曼哈顿启发式。
pub struct GroundNodeGenerator;

impl PathGenerator for GroundNodeGenerator {
    fn generate(
        &self,
        start: (i32, i32, i32),
        goal: (i32, i32, i32),
        is_solid: &dyn Fn(i32, i32, i32) -> bool,
    ) -> Option<PPath> {
        a_star_with(
            start,
            goal,
            is_solid,
            MAX_STEPS,
            &NEIGHBORS_4,
            Heuristic::Manhattan,
        )
    }
}

/// 高精度地面生成器：8 邻 + 对角代价，八向启发式。
pub struct PreciseGroundNodeGenerator;

impl PathGenerator for PreciseGroundNodeGenerator {
    fn generate(
        &self,
        start: (i32, i32, i32),
        goal: (i32, i32, i32),
        is_solid: &dyn Fn(i32, i32, i32) -> bool,
    ) -> Option<PPath> {
        a_star_with(
            start,
            goal,
            is_solid,
            MAX_STEPS,
            &NEIGHBORS_8,
            Heuristic::Octile,
        )
    }
}

/// 飞行生成器：26 邻 3D 全方向，欧氏启发式。
pub struct FlyingNodeGenerator;

impl PathGenerator for FlyingNodeGenerator {
    fn generate(
        &self,
        start: (i32, i32, i32),
        goal: (i32, i32, i32),
        is_solid: &dyn Fn(i32, i32, i32) -> bool,
    ) -> Option<PPath> {
        a_star_with(
            start,
            goal,
            is_solid,
            MAX_STEPS,
            &NEIGHBORS_26,
            Heuristic::Euclidean,
        )
    }
}

/// 水面生成器：4 邻水面航行（y 恒定为水面高度），曼哈顿启发式。
pub struct WaterNodeGenerator;

impl PathGenerator for WaterNodeGenerator {
    fn generate(
        &self,
        start: (i32, i32, i32),
        goal: (i32, i32, i32),
        is_solid: &dyn Fn(i32, i32, i32) -> bool,
    ) -> Option<PPath> {
        a_star_with(
            start,
            goal,
            is_solid,
            MAX_STEPS,
            &NEIGHBORS_4,
            Heuristic::Manhattan,
        )
    }
}

/// 到达节点判定距离平方（0.5 方块内视为已到达）。
const ARRIVE_DISTANCE_SQ: f64 = 0.25;

/// 沿已寻路径推进的导航器（纯结构，不依赖世界实例）。
#[derive(Debug)]
pub struct Navigator {
    /// 当前路径。
    pub path: Option<PPath>,
    /// 当前推进到的节点下标。
    pub index: usize,
    /// 移动速度（方块 / tick）。
    pub speed: f64,
}

impl Navigator {
    /// 以移动速度构造空导航器。
    pub fn new(speed: f64) -> Self {
        Self {
            path: None,
            index: 0,
            speed,
        }
    }

    /// 以指定生成器重新寻路。
    ///
    /// 契约偏差（T6 报告注明）：`PathGenerator::generate` 需要实心判定，
    /// 故在任务给定的 `set_target(start, goal, gen)` 基础上增加
    /// `is_solid` 参数。
    ///
    /// 寻路成功返回 `true`，失败（不可达 / 起终点相同但实心等）返回 `false`
    /// 并清空当前路径。
    pub fn set_target(
        &mut self,
        start: (i32, i32, i32),
        goal: (i32, i32, i32),
        is_solid: &dyn Fn(i32, i32, i32) -> bool,
        r#gen: &dyn PathGenerator,
    ) -> bool {
        match r#gen.generate(start, goal, is_solid) {
            Some(path) => {
                self.path = Some(path);
                self.index = 0;
                true
            }
            None => {
                self.path = None;
                self.index = 0;
                false
            }
        }
    }

    /// 沿路径推进并返回当前应移动到的目标点。
    ///
    /// 若当前位置已到达当前节点则推进 `index`；路径耗尽时清空路径并返回
    /// `None`。
    pub fn tick(&mut self, current: [f64; 3]) -> Option<[f64; 3]> {
        let [cx, cy, cz] = current;
        while let Some(node) = self
            .path
            .as_ref()
            .and_then(|p| p.nodes.get(self.index).copied())
        {
            let [tx, ty, tz] = [
                f64::from(node.0) + 0.5,
                f64::from(node.1),
                f64::from(node.2) + 0.5,
            ];
            let (dx, dy, dz) = (tx - cx, ty - cy, tz - cz);
            if dx * dx + dy * dy + dz * dz <= ARRIVE_DISTANCE_SQ {
                self.index = self.index.saturating_add(1);
            } else {
                return Some([tx, ty, tz]);
            }
        }
        self.path = None;
        None
    }

    /// 当前是否存在待跟随路径。
    pub fn has_path(&self) -> bool {
        self.path.is_some()
    }
}

/// 路径跟随器：由当前位置与目标点计算下一 tick 的速度向量。
pub trait NodeFollower: Send + Sync {
    /// 计算下一 tick 速度（方块 / tick）。
    fn next_velocity(&self, current: [f64; 3], target: [f64; 3], speed: f64) -> [f64; 3];
}

/// 水平归一化辅助：返回目标方向单位向量（y 轴为零）。
fn horizontal_direction(current: [f64; 3], target: [f64; 3]) -> Option<[f64; 3]> {
    let [cx, _, cz] = current;
    let [tx, _, tz] = target;
    let (dx, dz) = (tx - cx, tz - cz);
    let len = (dx * dx + dz * dz).sqrt();
    if len <= 1e-9 {
        None
    } else {
        Some([dx / len, 0.0, dz / len])
    }
}

/// 3D 归一化辅助：返回目标方向单位向量。
fn direction(current: [f64; 3], target: [f64; 3]) -> Option<[f64; 3]> {
    let [cx, cy, cz] = current;
    let [tx, ty, tz] = target;
    let (dx, dy, dz) = (tx - cx, ty - cy, tz - cz);
    let len = (dx * dx + dy * dy + dz * dz).sqrt();
    if len <= 1e-9 {
        None
    } else {
        Some([dx / len, dy / len, dz / len])
    }
}

/// 地面跟随器：水平面移动，竖直速度恒 0（重力由物理系统处理）。
pub struct GroundNodeFollower;

impl NodeFollower for GroundNodeFollower {
    fn next_velocity(&self, current: [f64; 3], target: [f64; 3], speed: f64) -> [f64; 3] {
        horizontal_direction(current, target)
            .map_or([0.0, 0.0, 0.0], |[dx, _, dz]| [dx * speed, 0.0, dz * speed])
    }
}

/// 飞行跟随器：3D 全方向移动。
pub struct FlyingNodeFollower;

impl NodeFollower for FlyingNodeFollower {
    fn next_velocity(&self, current: [f64; 3], target: [f64; 3], speed: f64) -> [f64; 3] {
        direction(current, target).map_or([0.0, 0.0, 0.0], |[dx, dy, dz]| {
            [dx * speed, dy * speed, dz * speed]
        })
    }
}

/// 水面跟随器：水平面移动（与地面一致，浮力由物理系统处理）。
pub struct WaterNodeFollower;

impl NodeFollower for WaterNodeFollower {
    fn next_velocity(&self, current: [f64; 3], target: [f64; 3], speed: f64) -> [f64; 3] {
        horizontal_direction(current, target)
            .map_or([0.0, 0.0, 0.0], |[dx, _, dz]| [dx * speed, 0.0, dz * speed])
    }
}

/// 无物理跟随器：直接以目标相对位移作为速度（“直飞”传送语义）。
pub struct NoPhysicsNodeFollower;

impl NodeFollower for NoPhysicsNodeFollower {
    fn next_velocity(&self, current: [f64; 3], target: [f64; 3], speed: f64) -> [f64; 3] {
        let [cx, cy, cz] = current;
        let [tx, ty, tz] = target;
        [(tx - cx) * speed, (ty - cy) * speed, (tz - cz) * speed]
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// 开放空间实心判定（全部可通行）。
    fn open_space(_x: i32, _y: i32, _z: i32) -> bool {
        false
    }

    #[test]
    fn a_star_finds_path_on_open_plane() {
        let path = a_star((0, 0, 0), (3, 0, 3), open_space, 4096).expect("开放平面应可达");
        assert_eq!(path.nodes.first(), Some(&(0, 0, 0)));
        assert_eq!(path.nodes.last(), Some(&(3, 0, 3)));
        // 路径节点逐段相邻。
        for w in path.nodes.windows(2) {
            let Some(a) = w.first().copied() else {
                continue;
            };
            let Some(b) = w.get(1).copied() else {
                continue;
            };
            assert!(a.0.abs_diff(b.0) <= 1 && a.1.abs_diff(b.1) <= 1 && a.2.abs_diff(b.2) <= 1);
        }
    }

    #[test]
    fn a_star_returns_none_when_unreachable() {
        // 用两道实心墙封闭目标（起点在墙外，目标被墙包围在 1×1 格内）。
        let is_solid = |x: i32, y: i32, z: i32| {
            (x == 2 && y == 0) || (x == 4 && y == 0) || (z == 2 && y == 0) || (z == 4 && y == 0)
        };
        // 目标 (3,0,3) 四周被封死，8 邻也无法到达。
        assert!(a_star((0, 0, 0), (3, 0, 3), is_solid, 4096).is_none());
    }

    #[test]
    fn a_star_returns_none_when_start_or_goal_solid() {
        let solid = |x: i32, _y: i32, _z: i32| x == 1;
        assert!(a_star((1, 0, 0), (3, 0, 0), solid, 4096).is_none());
        assert!(a_star((0, 0, 0), (1, 0, 0), solid, 4096).is_none());
    }

    #[test]
    fn a_star_bounded_by_max_steps() {
        // 远距离路径在极小步数上限下应返回 None（防爆）。
        assert!(a_star((0, 0, 0), (50, 0, 50), open_space, 10).is_none());
        // 足够步数上限则可达。
        assert!(a_star((0, 0, 0), (50, 0, 50), open_space, 100_000).is_some());
    }

    #[test]
    fn a_star_avoids_solid_obstacle() {
        // 一堵墙把 x=2..=3 封死（y=0），仅在 z=5 留出缺口，路径应从缺口绕行。
        let is_solid = |x: i32, y: i32, z: i32| (2..=3).contains(&x) && y == 0 && z != 5;
        let path = a_star((0, 0, 0), (5, 0, 0), is_solid, 4096).expect("应绕过障碍");
        assert!(path.len() > 2);
        // 所有路径节点不可落在实心墙上。
        for &(x, y, z) in &path.nodes {
            assert!(!is_solid(x, y, z));
        }
        // 路径应确实经过缺口 z=5 附近。
        assert!(
            path.nodes
                .iter()
                .any(|&(x, _, z)| (2..=3).contains(&x) && z == 5)
        );
    }

    #[test]
    fn ground_generator_finds_four_neighbor_path() {
        let r#gen = GroundNodeGenerator;
        let path = r#gen
            .generate((0, 5, 0), (4, 5, 0), &open_space)
            .expect("地面生成器应可达");
        assert_eq!(path.len(), 5); // 4 邻正交路径长度为 5 个节点
        for w in path.nodes.windows(2) {
            let Some(a) = w.first().copied() else {
                continue;
            };
            let Some(b) = w.get(1).copied() else {
                continue;
            };
            assert_eq!(a.1, b.1); // y 恒定
            assert!(a.0.abs_diff(b.0) + a.2.abs_diff(b.2) == 1); // 正交单步
        }
    }

    #[test]
    fn precise_ground_generator_allows_diagonal() {
        let r#gen = PreciseGroundNodeGenerator;
        let path = r#gen
            .generate((0, 5, 0), (2, 5, 2), &open_space)
            .expect("高精度地面生成器应可达");
        // 8 邻可走对角，路径节点数应不超过 4 邻的 5 个。
        assert!(path.len() <= 3, "对角路径应更短：{:?}", path.nodes);
    }

    #[test]
    fn flying_generator_moves_in_3d() {
        let r#gen = FlyingNodeGenerator;
        let path = r#gen
            .generate((0, 0, 0), (2, 3, 1), &open_space)
            .expect("飞行生成器应可达");
        assert_eq!(path.nodes.last(), Some(&(2, 3, 1)));
    }

    #[test]
    fn water_generator_keeps_surface_level() {
        let r#gen = WaterNodeGenerator;
        let path = r#gen
            .generate((0, 60, 0), (3, 60, 0), &open_space)
            .expect("水面生成器应可达");
        for &(_, y, _) in &path.nodes {
            assert_eq!(y, 60);
        }
    }

    #[test]
    fn navigator_advances_along_path() {
        let mut nav = Navigator::new(1.0);
        let r#gen = PreciseGroundNodeGenerator;
        assert!(nav.set_target((0, 0, 0), (3, 0, 0), &open_space, &r#gen));
        assert!(nav.has_path());
        // 站在起点：返回下一节点（节点 1 的块中心）。
        let first = nav.tick([0.5, 0.0, 0.5]).expect("应返回下一节点");
        let [fx, _, _] = first;
        assert!((fx - 1.5).abs() < 1e-9);
        // 推进到节点 1：返回节点 2。
        let second = nav.tick([1.5, 0.0, 0.5]).expect("应返回节点 2");
        let [sx, _, _] = second;
        assert!((sx - 2.5).abs() < 1e-9);
        // 到达终点后路径耗尽。
        let _ = nav.tick([2.5, 0.0, 0.5]);
        let _ = nav.tick([3.5, 0.0, 0.5]);
        assert!(!nav.has_path());
    }

    #[test]
    fn navigator_rejects_unreachable_target() {
        let mut nav = Navigator::new(1.0);
        let r#gen = GroundNodeGenerator;
        let is_solid = |x: i32, y: i32, z: i32| {
            (x == 2 && y == 0) || (x == 4 && y == 0) || (z == 2 && y == 0) || (z == 4 && y == 0)
        };
        assert!(!nav.set_target((0, 0, 0), (3, 0, 3), &is_solid, &r#gen));
        assert!(!nav.has_path());
        assert_eq!(nav.tick([0.5, 0.0, 0.5]), None);
    }

    #[test]
    fn ground_follower_moves_horizontally() {
        let follower = GroundNodeFollower;
        let vel = follower.next_velocity([0.0, 64.0, 0.0], [10.0, 64.0, 0.0], 2.0);
        let [vx, vy, vz] = vel;
        assert!((vx - 2.0).abs() < 1e-9);
        assert_eq!(vy, 0.0);
        assert_eq!(vz, 0.0);
    }

    #[test]
    fn flying_follower_moves_in_3d() {
        let follower = FlyingNodeFollower;
        let vel = follower.next_velocity([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 1.0);
        let len = (vel[0] * vel[0] + vel[1] * vel[1] + vel[2] * vel[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-9);
    }

    #[test]
    fn followers_return_zero_when_already_at_target() {
        let ground = GroundNodeFollower;
        let flying = FlyingNodeFollower;
        assert_eq!(
            ground.next_velocity([1.0, 0.0, 1.0], [1.0, 0.0, 1.0], 5.0),
            [0.0, 0.0, 0.0]
        );
        assert_eq!(
            flying.next_velocity([1.0, 1.0, 1.0], [1.0, 1.0, 1.0], 5.0),
            [0.0, 0.0, 0.0]
        );
    }
}
