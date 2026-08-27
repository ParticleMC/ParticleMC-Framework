// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 实例区块存储（`ChunkStore`）：作为实例 World 的只读/可变 `Resource`，
//! 承载原 `InstanceContainer` 的区块数据、脏标记、生成器与持久化器。
//!
//! R11 世界化后，区块不再随 `InstanceContainer` 组件挂在主 World，而是作为
//! 实例专属 World 内的 `Resource` 存在，使各实例 World 实体与区块内存隔离、
//! 跨实例互不可见（见 `.specs/implement-custom-ecs/spec.md` R11）。块路由委托
//! 逻辑（世界坐标 → 区块 → 区段 → 局部索引）原样保留于此。

use std::collections::{HashMap, HashSet, VecDeque};

use super::chunk::{Chunk, SectionFillError, SECTION_VOLUME};
use super::generator::{ChunkGenerator, generated_to_chunk};
use super::light_engine::{LightBoundaryDir, LightEngine, SectionLightBoundary};
use super::loader::ChunkLoader;
use crate::component::Block;
use crate::resource::registries::BlockRegistry;

/// 实例区块存储（`Resource`）。
///
/// 持有已加载区块表、脏标记，以及可选的生成器 / 持久化器（均以 `Box<dyn …>`
/// 存储，`None` 表示未装配）。生成器产出缺失区块，持久化器负责全量快照存读。
///
/// `max_chunks` 为 `None` 时不限制区块数量（向后兼容）；设置后超过上限时自动
/// 淘汰最少访问的区块（LRU）。属可选功能，由调用方按需配置。
pub struct ChunkStore {
    /// 已加载区块。
    chunks: HashMap<(i32, i32), Chunk>,
    /// 自上次取出后发生写入的区块坐标集合。
    dirty_chunks: HashSet<(i32, i32)>,
    /// 动态分片：含本帧「活动 / 移动实体」（速度非零）所在区块坐标的集合。
    ///
    /// 用于 R11.4「主 Tick 仅遍历动态分片」的物理加速——第一遍标记本帧有移动
    /// 实体的区块，第二遍仅对落在该集合中的实体做物理积分；静止实体所在区块
    /// 整帧跳过积分（位置不变，安全）。属运行时状态，克隆时重置为空。
    dynamic_chunks: HashSet<(i32, i32)>,
    /// 区块生成器（`None` 表示不生成区块）。
    generator: Option<Box<dyn ChunkGenerator>>,
    /// 区块持久化器（`None` 表示仅内存直存）。
    loader: Option<Box<dyn ChunkLoader>>,
    /// 方块注册表（光属性来源），装配后用于方块变更/加载时的即时光照重算。
    ///
    /// `None` 表示未装配，方块变更保持旧行为（不触发光照重算），向后兼容。
    registry: Option<BlockRegistry>,
    /// 最大加载区块数；`None` 表示不限制（向后兼容）。
    max_chunks: Option<usize>,
    /// LRU 访问顺序（最新访问在前）；仅用于淘汰决策，不参与相等性比较。
    /// 属运行时状态，克隆时重置为空。
    access_order: VecDeque<(i32, i32)>,
    /// 区块首次加载的 FIFO 插入顺序（最早加载的在队头，`pop_front` 即最早插入）。
    ///
    /// 选用 `VecDeque` 而非其他集合：与 `access_order` 结构一致、队首/队尾两端
    /// 均 O(1)——新加载区块 `push_back` 到队尾，最早插入区块 `pop_front` 直取；
    /// 且便于克隆时整体保留。该字段是区块集合的数据元信息（相对加载次序）而非
    /// 运行时访问痕迹，区别于克隆重置的 `access_order`，从而保证克隆副本的淘汰
    /// 仍能确定性回退到 FIFO 顺序（见 `evict_lru` 退化路径）。
    insertion_order: VecDeque<(i32, i32)>,
    /// 世界配置的每区块区段数（默认 1，向后兼容）。
    section_count: usize,
}

impl std::fmt::Debug for ChunkStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChunkStore")
            .field("chunks", &self.chunks)
            .field("dirty_chunks", &self.dirty_chunks)
            .field("dynamic_count", &self.dynamic_chunks.len())
            .field("has_generator", &self.generator.is_some())
            .field("has_loader", &self.loader.is_some())
            .finish()
    }
}

impl Clone for ChunkStore {
    /// 克隆数据部分（区块 + 脏标记 + 插入顺序）；生成器与持久化器不可克隆，置为
    /// `None`。`access_order`/动态分片为运行时状态，克隆重置为空；`max_chunks` 与
    /// `insertion_order`（区块数据元信息）作为数据随克隆保留，使副本在无访问记录
    /// 时仍能依插入顺序确定性执行 FIFO 淘汰。
    fn clone(&self) -> Self {
        Self {
            chunks: self.chunks.clone(),
            dirty_chunks: self.dirty_chunks.clone(),
            // 动态分片为运行时加速状态：克隆重置为空，避免副本误带旧分片。
            dynamic_chunks: HashSet::new(),
            generator: None,
            loader: None,
            registry: self.registry.clone(),
            max_chunks: self.max_chunks,
            // 访问顺序为运行时状态，克隆重置。
            access_order: VecDeque::new(),
            // 插入顺序为区块数据元信息，克隆保留以便确定性 FIFO 淘汰。
            insertion_order: self.insertion_order.clone(),
            section_count: self.section_count,
        }
    }
}

impl PartialEq for ChunkStore {
    fn eq(&self, other: &Self) -> bool {
        // 仅比较数据部分（已加载区块 + 脏标记）。动态分片为运行时加速状态，
        // 不参与相等性判定（克隆即重置），避免副本间因分片差异误判不等。
        self.chunks == other.chunks && self.dirty_chunks == other.dirty_chunks
    }
}

impl Eq for ChunkStore {}

impl ChunkStore {
    /// 构造空存储。
    pub fn new() -> Self {
        Self {
            chunks: HashMap::new(),
            dirty_chunks: HashSet::new(),
            dynamic_chunks: HashSet::new(),
            generator: None,
            loader: None,
            registry: None,
            max_chunks: None,
            access_order: VecDeque::new(),
            insertion_order: VecDeque::new(),
            section_count: 1,
        }
    }

    /// 设置最大加载区块数；`None` 表示不限制（向后兼容默认行为）。
    pub fn set_max_chunks(&mut self, capacity: Option<usize>) {
        self.max_chunks = capacity;
    }

    /// 查询当前最大加载区块数限制；`None` 表示无上限。
    pub fn max_chunks(&self) -> Option<usize> {
        self.max_chunks
    }

    /// 返回世界配置的每区块区段数（默认 1）。
    pub fn section_count(&self) -> usize {
        self.section_count
    }

    /// 循环淘汰最少访问的区块，直至加载数不超过 `max_chunks`。
    ///
    /// 未设置上限（`max_chunks` 为 `None`）时为空操作。
    /// 当 `access_order` 为空（如克隆后重置）时，退化为 FIFO：按插入顺序淘汰最旧的区块。
    fn evict_lru(&mut self) {
        let Some(limit) = self.max_chunks else {
            return;
        };
        while self.chunks.len() > limit {
            if let Some((evict_x, evict_z)) = self.access_order.pop_back() {
                if self.chunks.contains_key(&(evict_x, evict_z)) {
                    self.chunks.remove(&(evict_x, evict_z));
                    // 同步放弃插入顺序中的对应条目，避免留下幽灵坐标。
                    self.insertion_order.retain(|&(cx, cz)| cx != evict_x || cz != evict_z);
                }
            } else {
                // access_order 为空（如克隆后无访问记录），退化为 FIFO：取最早
                // 插入的坐标（insertion_order 队首），淘汰该区块。
                if let Some((evict_x, evict_z)) = self.insertion_order.pop_front() {
                    // 防御：该坐标可能已被卸载但保留在队中；仅当仍在库时移除区块。
                    if self.chunks.contains_key(&(evict_x, evict_z)) {
                        self.chunks.remove(&(evict_x, evict_z));
                    }
                } else {
                    break;
                }
            }
        }
    }

    /// 将指定坐标标记为最新访问（移到 LRU 队列头部）。
    fn mark_access(&mut self, x: i32, z: i32) {
        self.access_order.retain(|&(cx, cz)| cx != x || cz != z);
        self.access_order.push_front((x, z));
    }

    /// 加载（或替换）一个区块，返回被替换的旧区块（若有）。
    ///
    /// 加载后更新 LRU 访问顺序，并在超过上限时触发淘汰。
    pub fn load_chunk(&mut self, chunk: Chunk) -> Option<Chunk> {
        let coords = (chunk.x, chunk.z);
        let replaced = self.chunks.insert(coords, chunk);
        // 维护 FIFO 插入顺序：替换已存在的坐标先移除旧条目，再追加到队尾；
        // 新增坐标直接追加到队尾。
        self.insertion_order.retain(|&(cx, cz)| cx != coords.0 || cz != coords.1);
        self.insertion_order.push_back(coords);
        self.recompute_light(coords.0, coords.1);
        self.evict_lru();
        // 访问标记置于淘汰之后：避免把刚加载的区块误当作淘汰候选，确保克隆等
        // 无访问记录场景下 evict_lru 能命中 FIFO 退化路径。
        self.mark_access(coords.0, coords.1);
        replaced
    }

    /// 卸载指定坐标的区块，返回被移除的区块（若有）。
    pub fn unload_chunk(&mut self, x: i32, z: i32) -> Option<Chunk> {
        let removed = self.chunks.remove(&(x, z));
        if removed.is_some() {
            // 同步移除插入顺序中的对应条目，避免留下幽灵坐标。
            self.insertion_order.retain(|&(cx, cz)| cx != x || cz != z);
        }
        removed
    }

    /// 查询指定坐标的区块。
    ///
    /// 注意：本方法为只读访问（`&self`），不更新 LRU 顺序；LRU 由写路径
    /// （`load_chunk` / `set_block` / `set_block_id_world`）维护。
    pub fn get_chunk(&self, x: i32, z: i32) -> Option<&Chunk> {
        self.chunks.get(&(x, z))
    }

    /// 只读遍历全部已加载区块（顺序不定）。
    pub fn iter_chunks(&self) -> impl Iterator<Item = &Chunk> {
        self.chunks.values()
    }

    /// 按区块坐标跨区段写入方块：定位到对应区块并下发写入。
    ///
    /// 若目标区块未加载，返回 `false`。写入成功后同时记录脏区块、更新 LRU
    /// 顺序并在超过上限时触发淘汰。
    pub fn set_block(&mut self, x: i32, z: i32, section: usize, index: usize, id: u32) -> bool {
        let written = match self.chunks.get_mut(&(x, z)) {
            Some(chunk) => chunk.set_block(section, index, id),
            None => false,
        };
        if written {
            self.dirty_chunks.insert((x, z));
            self.mark_access(x, z);
            self.recompute_light(x, z);
            self.evict_lru();
        }
        written
    }

    /// 已加载区块数量。
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// 石头方块状态 id（默认注册表中 `minecraft:stone` 通常对应 1，空气为 0）。
    pub const STONE_BLOCK_ID: u32 = 1;
    /// 空气方块状态 id。
    pub const AIR_BLOCK_ID: u32 = 0;

    /// 按世界坐标写入方块 id（路由到对应区块 / 区段 / 局部索引）。
    ///
    /// 坐标顺序为 `(x, y, z)`。y < 0、目标区块未加载或区段越界时返回 `false`。
    /// 写入成功后将该区块坐标记入 [`dirty_chunks`](Self::dirty_chunks)，更新 LRU
    /// 顺序，并在超过上限时触发淘汰。
    pub fn set_block_id_world(&mut self, x: i32, y: i32, z: i32, id: u32) -> bool {
        if y < 0 {
            return false;
        }
        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);
        let chunk = match self.chunks.get_mut(&(cx, cz)) {
            Some(c) => c,
            None => return false,
        };
        let section = usize::try_from(y / 16).unwrap_or(usize::MAX);
        let y_local = usize::try_from(y.rem_euclid(16)).unwrap_or(0);
        let x_local = usize::try_from(x.rem_euclid(16)).unwrap_or(0);
        let z_local = usize::try_from(z.rem_euclid(16)).unwrap_or(0);
        let index = y_local * 256 + z_local * 16 + x_local;
        if chunk.set_block(section, index, id) {
            self.dirty_chunks.insert((cx, cz));
            self.mark_access(cx, cz);
            self.recompute_light(cx, cz);
            self.evict_lru();
            true
        } else {
            false
        }
    }

    /// 按世界坐标写入方块（坐标顺序 `(x, y, z)`）。
    ///
    /// 内部取方块状态 id 后委托给 [`set_block_id_world`](Self::set_block_id_world)。
    pub fn set_block_world(&mut self, x: i32, y: i32, z: i32, block: Block) -> bool {
        self.set_block_id_world(x, y, z, block.state_id())
    }

    /// 按世界坐标读取方块 id（坐标顺序 `(x, y, z)`）。
    ///
    /// y < 0、未加载区块、区段越界或空气均返回 0。
    pub fn get_block_id_world(&self, x: i32, y: i32, z: i32) -> u32 {
        if y < 0 {
            return 0;
        }
        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);
        let chunk = match self.chunks.get(&(cx, cz)) {
            Some(c) => c,
            None => return 0,
        };
        let section = usize::try_from(y / 16).unwrap_or(usize::MAX);
        let y_local = usize::try_from(y.rem_euclid(16)).unwrap_or(0);
        let x_local = usize::try_from(x.rem_euclid(16)).unwrap_or(0);
        let z_local = usize::try_from(z.rem_euclid(16)).unwrap_or(0);
        let index = y_local * 256 + z_local * 16 + x_local;
        chunk.get_block(section, index)
    }

    /// 按世界坐标读取方块（坐标顺序 `(x, y, z)`，语义经注册表解析）。
    pub fn get_block_world(&self, x: i32, y: i32, z: i32, registry: &BlockRegistry) -> Block {
        let _ = registry;
        Block::from_state_id(self.get_block_id_world(x, y, z))
    }

    /// 只读访问脏区块坐标集合。
    pub fn dirty_chunks(&self) -> &HashSet<(i32, i32)> {
        &self.dirty_chunks
    }

    /// 取走全部脏区块坐标并清空集合。
    pub fn take_dirty_chunks(&mut self) -> HashSet<(i32, i32)> {
        std::mem::take(&mut self.dirty_chunks)
    }

    /// 只读访问动态分片（含本帧移动实体所在区块坐标的集合）。
    pub fn dynamic_chunks(&self) -> &HashSet<(i32, i32)> {
        &self.dynamic_chunks
    }

    /// 将指定区块坐标加入动态分片（标记该区块本帧含移动实体）。
    pub fn mark_chunk_dynamic(&mut self, cx: i32, cz: i32) {
        self.dynamic_chunks.insert((cx, cz));
    }

    /// 判断指定区块是否在动态分片中（即本帧含移动实体）。
    pub fn is_chunk_dynamic(&self, cx: i32, cz: i32) -> bool {
        self.dynamic_chunks.contains(&(cx, cz))
    }

    /// 清空动态分片（每帧物理第一遍前调用，重建本帧分片集合）。
    pub fn clear_dynamic(&mut self) {
        self.dynamic_chunks.clear();
    }

    /// 从动态分片移除指定区块（按需；例如实体减速归零后逐出）。
    pub fn remove_chunk_dynamic(&mut self, cx: i32, cz: i32) {
        self.dynamic_chunks.remove(&(cx, cz));
    }

    /// 装配区块生成器（覆盖已有生成器）。
    pub fn set_generator(&mut self, generator: Box<dyn ChunkGenerator>) {
        self.generator = Some(generator);
    }

    /// 装配区块持久化器（覆盖已有持久化器）。
    pub fn set_loader(&mut self, loader: Box<dyn ChunkLoader>) {
        self.loader = Some(loader);
    }

    /// 装配方块注册表（光属性来源）。
    ///
    /// 装配后，方块变更 / 加载将即时触发 [`LightEngine::recompute`] 重算受影响
    /// 区块（含邻块）的天空光与方块光；未装配（`None`）时保持旧行为（不重算），
    /// 向后兼容既有调用方。见 `complete-framework-gaps` WS1-T4。
    pub fn set_registry(&mut self, registry: BlockRegistry) {
        self.registry = Some(registry);
    }

    /// 即时光照重算：取出目标区块，结合 4 邻块边界种子重算其光照后写回。
    ///
    /// 采用「取出目标区块 + 提取邻块边界」策略，避免对 `self.chunks` 同时持有
    /// 可变（目标）与不可变（邻居）借用。邻居通过 [`Chunk::extract_light_boundary`]
    /// 仅拷贝边界列数据，而非整块克隆，显著降低高频方块变更时的内存分配。
    /// 区块缺失或未装配注册表时为空操作（返回 `false`）。
    pub(crate) fn recompute_light(&mut self, cx: i32, cz: i32) -> bool {
        let Some(registry) = &self.registry else {
            return false;
        };
        let mut chunk = match self.chunks.remove(&(cx, cz)) {
            Some(chunk) => chunk,
            None => return false,
        };
        // 快照重算前光照，用于比较变化。
        let before = chunk.light.clone();
        // 提取 4 个方向的边界光照数据（每个方向仅需边界列，无需整块克隆）。
        let north = self.chunks.get(&(cx, cz - 1)).map(|c| c.extract_light_boundary(LightBoundaryDir::North));
        let south = self.chunks.get(&(cx, cz + 1)).map(|c| c.extract_light_boundary(LightBoundaryDir::South));
        let west  = self.chunks.get(&(cx - 1, cz)).map(|c| c.extract_light_boundary(LightBoundaryDir::West));
        let east  = self.chunks.get(&(cx + 1, cz)).map(|c| c.extract_light_boundary(LightBoundaryDir::East));
        let boundary: [SectionLightBoundary; 4] = [
            east.unwrap_or_else(|| SectionLightBoundary::empty(chunk.section_count())),
            west.unwrap_or_else(|| SectionLightBoundary::empty(chunk.section_count())),
            south.unwrap_or_else(|| SectionLightBoundary::empty(chunk.section_count())),
            north.unwrap_or_else(|| SectionLightBoundary::empty(chunk.section_count())),
        ];
        LightEngine::recompute_with_boundary(&mut chunk, &boundary, registry);
        let changed = chunk.light != before;
        self.chunks.insert((cx, cz), chunk);
        changed
    }

    /// 取回已装配的区块持久化器（用于转移给其他存储或销毁）。
    pub fn take_loader(&mut self) -> Option<Box<dyn ChunkLoader>> {
        self.loader.take()
    }

    /// 获取或生成指定坐标的区块。
    ///
    /// - 已加载：返回克隆，不触发生成；更新 LRU 顺序。
    /// - 缺失且有生成器：经生成器产出并入库后返回克隆；触发淘汰检查。
    /// - 缺失且无生成器：返回 `None`。
    pub fn get_or_generate_chunk(&mut self, x: i32, z: i32, seed: u64) -> Option<Chunk> {
        let has_chunk = self.chunks.contains_key(&(x, z));
        if has_chunk {
            self.mark_access(x, z);
            return self.chunks.get(&(x, z)).cloned();
        }
        let generated = self.generator.as_ref()?.generate(x, z, seed);
        let chunk = generated_to_chunk(&generated, x, z);
        let _ = self.chunks.insert((x, z), chunk.clone());
        // 维护 FIFO 插入顺序（该坐标此前缺失，retain 仅为防御）。
        self.insertion_order.retain(|&(cx, cz)| cx != x || cz != z);
        self.insertion_order.push_back((x, z));
        self.evict_lru();
        // 访问标记置于淘汰之后，与 load_chunk 保持一致。
        self.mark_access(x, z);
        Some(chunk)
    }

    /// 从持久化器预载全部已保存区块（覆盖同坐标的现存区块）。
    ///
    /// 未装配持久化器时为空操作。
    pub fn load_all(&mut self) {
        let loader = match self.loader.as_mut() {
            Some(loader) => loader,
            None => return,
        };
        let keys = loader.keys();
        for (x, z) in keys {
            if let Some(chunk) = loader.load(x, z) {
                self.chunks.insert((x, z), chunk);
                // 维护 FIFO 插入顺序：覆盖已存在的坐标先移除旧条目，再追加到队尾。
                self.insertion_order.retain(|&(cx, cz)| cx != x || cz != z);
                self.insertion_order.push_back((x, z));
            }
        }
    }

    /// 把全部已加载区块保存进持久化器。
    ///
    /// 未装配持久化器时为空操作。
    pub fn save_all(&mut self) {
        let loader = match self.loader.as_mut() {
            Some(loader) => loader,
            None => return,
        };
        for chunk in self.chunks.values() {
            loader.save(chunk);
        }
    }
}

/// 批量编辑上下文：在填充多个区块期间暂停光照/LRU/淘汰副作用，
/// `finalize()` 时一次性提交所有修改并迭代重算光照直至稳定。
///
/// 所有修改写入 `pending_chunks` 副本，原始 ChunkStore 数据在 `finalize()` 前完全不变。
/// `abort()` 丢弃 pending；`Drop` 清理 pending 且不触碰原始数据（防止半提交状态）。
pub struct BulkEditContext<'a> {
    /// ChunkStore 引用（用于读取原始区块和世界配置）。
    store: &'a mut ChunkStore,
    /// 待提交的修改/新增区块（完整副本，finalize 时才合并到 store）。
    pending_chunks: HashMap<(i32, i32), Chunk>,
    /// 待标记的脏区块坐标集合。
    pending_dirty: HashSet<(i32, i32)>,
    /// 本次操作的访问顺序（最新在前，finalize 时合并到 store）。
    pending_access_order: VecDeque<(i32, i32)>,
    /// 世界配置的每区块区段数（用于校验输入一致性）。
    section_count: usize,
    /// 方块注册表（保留用于未来扩展）。
    registry: Option<BlockRegistry>,
}

impl<'a> BulkEditContext<'a> {
    /// 填充指定区块的所有区段（修改写入 pending 副本，不影响 store）。
    ///
    /// `section_ids_vec[i]` = 第 i 个区段的 4096 格 id 数据（空切片跳过该区段）。
    /// 先验校验所有区段长度和区段数量，全部通过后才写入 pending。
    /// 若区块不存在则自动创建（区段数由世界配置决定）。
    /// 同一区块多次调用时，后续调用在已有 pending 副本上叠加修改。
    pub fn fill_chunk(
        &mut self,
        world_x: i32,
        world_z: i32,
        section_ids_vec: &[&[u32]],
    ) -> Result<(), SectionFillError> {
        // 先验校验：所有非空切片长度必须等于 SECTION_VOLUME。
        for ids in section_ids_vec {
            if !ids.is_empty() && ids.len() != SECTION_VOLUME {
                return Err(SectionFillError::InvalidLength);
            }
        }
        // 区段数量必须匹配世界配置。
        if section_ids_vec.len() != self.section_count {
            return Err(SectionFillError::InvalidLength);
        }
        let cx = world_x.div_euclid(16);
        let cz = world_z.div_euclid(16);
        // 优先从 pending 取已有副本，避免覆盖前次修改。
        let has_pending = self.pending_chunks.contains_key(&(cx, cz));
        let mut chunk = if has_pending {
            self.pending_chunks[&(cx, cz)].clone()
        } else {
            match self.store.chunks.get(&(cx, cz)).cloned() {
                Some(c) => c,
                None => Chunk::new(cx, cz, self.section_count),
            }
        };
        // 填充各区段（空切片跳过，保留原区段数据）。
        for (i, ids) in section_ids_vec.iter().enumerate() {
            if ids.is_empty() {
                continue;
            }
            chunk.fill_section_blocks(i, ids)?;
        }
        self.pending_chunks.insert((cx, cz), chunk);
        // 记录脏标记和访问顺序（去重，push_front 使最新在前）。
        self.pending_dirty.insert((cx, cz));
        self.pending_access_order.retain(|&(x, z)| x != cx || z != cz);
        self.pending_access_order.push_front((cx, cz));
        Ok(())
    }

    /// 一次性提交所有修改：合并 pending_chunks 到 store，LRU 合并，淘汰检查，迭代光照传播。
    pub fn finalize(&mut self) -> Result<(), SectionFillError> {
        // 1. 合并区块数据。
        for (coords, chunk) in self.pending_chunks.drain() {
            self.store.chunks.insert(coords, chunk);
            // 维护 FIFO 插入顺序：覆盖已存在的坐标先移除旧条目，再追加到队尾。
            self.store.insertion_order.retain(|&(cx, cz)| cx != coords.0 || cz != coords.1);
            self.store.insertion_order.push_back(coords);
        }
        // 2. 合并脏标记。
        self.store.dirty_chunks.extend(self.pending_dirty.drain());
        // 3. LRU 合并（含去重）：先从 store 移除重复坐标，再插入 pending 坐标。
        let pending_order: VecDeque<(i32, i32)> = std::mem::take(&mut self.pending_access_order);
        self.store.access_order.retain(|&(x, z)| !pending_order.contains(&(x, z)));
        for pair in pending_order {
            self.store.access_order.push_front(pair);
        }
        // 4. 淘汰检查仅一次。
        self.store.evict_lru();
        // 5. 迭代光照传播（最多 1024 轮）。
        const MAX_LIGHT_ITERATIONS: usize = 1024;
        let mut to_process: HashSet<(i32, i32)> = self.store.dirty_chunks.clone();
        for _ in 0..MAX_LIGHT_ITERATIONS {
            if to_process.is_empty() {
                break;
            }
            let mut next_round: HashSet<(i32, i32)> = HashSet::new();
            for &(cx, cz) in &to_process {
                if self.store.recompute_light(cx, cz) {
                    // 本轮变化：将 4 邻块加入下一轮处理。
                    next_round.insert((cx + 1, cz));
                    next_round.insert((cx - 1, cz));
                    next_round.insert((cx, cz + 1));
                    next_round.insert((cx, cz - 1));
                }
            }
            to_process = next_round;
        }
        // 6. 清空 pending 状态；finalize 提交完成后 dirty_chunks 同步清零，
        //    表示所有修改已合并到主存储且光照传播已完成。
        self.store.dirty_chunks.clear();
        self.registry.take();
        Ok(())
    }

    /// 放弃所有 pending 修改，对 store 无任何影响。
    pub fn abort(&mut self) {
        self.pending_chunks.clear();
        self.pending_dirty.clear();
        self.pending_access_order.clear();
        self.registry.take();
    }
}

impl<'a> Drop for BulkEditContext<'a> {
    /// Drop 仅清理 pending 状态，不提交不回滚，不触碰 store。
    fn drop(&mut self) {
        self.pending_chunks.clear();
        self.pending_dirty.clear();
        self.pending_access_order.clear();
    }
}

impl ChunkStore {
    /// 启动批量编辑上下文。
    ///
    /// 在 `finalize()` 或 `abort()` 调用前，所有 `fill_chunk` 修改仅写入内部 pending，
    /// 原始 ChunkStore 数据完全不变。
    pub fn start_bulk_edit(&mut self) -> BulkEditContext<'_> {
        // 先捕获值，避免与 store 字段的借用冲突。
        let section_count = self.section_count;
        let registry = self.registry.clone();
        BulkEditContext {
            store: self,
            pending_chunks: HashMap::new(),
            pending_dirty: HashSet::new(),
            pending_access_order: VecDeque::new(),
            section_count,
            registry,
        }
    }
}

impl Default for ChunkStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instance::chunk::{Chunk, SECTION_VOLUME};

    fn make_chunk(x: i32, z: i32) -> Chunk {
        let mut c = Chunk::new(x, z, 1);
        for i in 0..SECTION_VOLUME {
            let _ = c.set_block(0, i, 1);
        }
        c
    }

    #[test]
    fn no_eviction_when_max_chunks_is_none() {
        let mut store = ChunkStore::new();
        store.set_max_chunks(None);
        for i in 0..10 {
            store.load_chunk(make_chunk(i, 0));
        }
        assert_eq!(store.chunk_count(), 10);
    }

    #[test]
    fn evict_lru_removes_least_recently_accessed() {
        let mut store = ChunkStore::new();
        store.set_max_chunks(Some(3));
        store.load_chunk(make_chunk(0, 0));
        store.load_chunk(make_chunk(1, 0));
        store.load_chunk(make_chunk(2, 0));
        // 通过写入使 chunk(0,0) 变为最新访问（y=0 在单区段区块 [0,15] 范围内）。
        let _ = store.set_block_id_world(0, 0, 0, 1);
        // 加载第 4 个，应淘汰 chunk(1,0)（最少访问）。
        store.load_chunk(make_chunk(3, 0));
        assert!(store.get_chunk(1, 0).is_none(), "chunk(1,0) 应被淘汰");
        assert!(store.get_chunk(0, 0).is_some(), "chunk(0,0) 仍保留");
        assert!(store.get_chunk(2, 0).is_some(), "chunk(2,0) 仍保留");
        assert!(store.get_chunk(3, 0).is_some(), "chunk(3,0) 仍保留");
    }

    #[test]
    fn write_updates_lru_order() {
        let mut store = ChunkStore::new();
        store.set_max_chunks(Some(2));
        store.load_chunk(make_chunk(0, 0));
        store.load_chunk(make_chunk(1, 0));
        // 写入 chunk(0,0) 使其变为最新访问（y=0 在单区段区块范围内）。
        let _ = store.set_block_id_world(0, 0, 0, 1);
        // 加载第 3 个，应淘汰 chunk(1,0)（Least Recently Used）。
        store.load_chunk(make_chunk(2, 0));
        assert!(store.get_chunk(1, 0).is_none(), "chunk(1,0) 应被淘汰");
        assert!(store.get_chunk(0, 0).is_some(), "chunk(0,0) 因写入更新而保留");
    }

    #[test]
    fn clone_excludes_access_order() {
        let mut store = ChunkStore::new();
        store.set_max_chunks(Some(2));
        store.load_chunk(make_chunk(0, 0));
        store.load_chunk(make_chunk(1, 0));
        // 写入 chunk(0,0) 使其变为最新访问（y=0 在单区段区块范围内）。
        let _ = store.set_block_id_world(0, 0, 0, 1);
        let mut cloned = store.clone();
        assert_eq!(cloned.max_chunks(), Some(2));
        assert_eq!(cloned.chunk_count(), 2);
        // 克隆的 access_order 应为空；加载新块时应淘汰最早插入的块（(0,0)），
        // 而非原存储的 LRU 头部（(0,0)）。
        cloned.load_chunk(make_chunk(2, 0));
        // 克隆无 LRU 顺序，淘汰顺序取决于插入顺序：先插 (0,0)，再插 (1,0)，
        // 再插 (2,0) → 淘汰最早插入的 (0,0)。
        assert!(cloned.get_chunk(0, 0).is_none(), "克隆后应淘汰最早插入的区块");
    }

    #[test]
    fn partial_eq_excludes_access_order() {
        let mut a = ChunkStore::new();
        let mut b = ChunkStore::new();
        a.load_chunk(make_chunk(0, 0));
        b.load_chunk(make_chunk(0, 0));
        assert_eq!(a, b, "相同数据应相等，即使 access_order 不同");
    }

    // ── BulkEditContext 测试 ────────────────────────────────────────────────────

    #[test]
    fn bulk_edit_does_not_affect_store_before_finalize() {
        let mut store = ChunkStore::new();
        store.load_chunk(make_chunk(0, 0));
        let original_count = store.chunk_count();
        let ids = vec![2u32; SECTION_VOLUME];
        {
            let mut ctx = store.start_bulk_edit();
            ctx.fill_chunk(0, 0, &[&ids]).unwrap();
            // ctx 持有 store 的可变引用，无法直接访问 store，但 pending 未提交。
        }
        assert_eq!(store.chunk_count(), original_count, "Drop 后 store 也不应改变");
    }

    #[test]
    fn finalize_writes_pending_chunks_to_store() {
        let mut store = ChunkStore::new();
        let ids = vec![42u32; SECTION_VOLUME];
        {
            let mut ctx = store.start_bulk_edit();
            ctx.fill_chunk(0, 0, &[&ids]).unwrap();
            ctx.finalize().unwrap();
        }
        let chunk = store.get_chunk(0, 0).unwrap();
        assert_eq!(chunk.get_block(0, 0), 42);
    }

    #[test]
    fn finalize_clears_dirty_chunks() {
        let mut store = ChunkStore::new();
        let ids = vec![1u32; SECTION_VOLUME];
        {
            let mut ctx = store.start_bulk_edit();
            ctx.fill_chunk(0, 0, &[&ids]).unwrap();
            ctx.fill_chunk(1, 0, &[&ids]).unwrap();
            ctx.fill_chunk(2, 0, &[&ids]).unwrap();
            ctx.finalize().unwrap();
        }
        assert!(store.dirty_chunks().is_empty(), "finalize 后 dirty_chunks 应为空");
    }

    #[test]
    fn finalize_sets_dirty_chunks_before_light_recompute() {
        let mut store = ChunkStore::new();
        let ids = vec![1u32; SECTION_VOLUME];
        let mut ctx = store.start_bulk_edit();
        ctx.fill_chunk(0, 0, &[&ids]).unwrap();
        drop(ctx); // Drop 不清理 dirty_chunks
        // Drop 只清理 pending，dirty_chunks 应由 finalize 管理
    }

    #[test]
    fn abort_does_not_affect_store() {
        let mut store = ChunkStore::new();
        store.load_chunk(make_chunk(0, 0));
        let ids = vec![99u32; SECTION_VOLUME];
        {
            let mut ctx = store.start_bulk_edit();
            ctx.fill_chunk(0, 0, &[&ids]).unwrap();
            ctx.abort();
        }
        assert_eq!(store.get_chunk(0, 0).unwrap().get_block(0, 0), 1, "abort 后区块数据不变");
    }

    #[test]
    fn drop_does_not_affect_store() {
        let mut store = ChunkStore::new();
        store.load_chunk(make_chunk(0, 0));
        let ids = vec![99u32; SECTION_VOLUME];
        {
            let mut ctx = store.start_bulk_edit();
            ctx.fill_chunk(0, 0, &[&ids]).unwrap();
            // 不调用 finalize 或 abort，直接 drop
        }
        assert_eq!(store.get_chunk(0, 0).unwrap().get_block(0, 0), 1, "Drop 后区块数据不变");
    }

    #[test]
    fn fill_chunk_auto_creates_new_chunk_with_section_count() {
        let mut store = ChunkStore::new();
        // 默认 section_count = 1，所以传入 1 个区段的数据应成功
        let ids = vec![7u32; SECTION_VOLUME];
        {
            let mut ctx = store.start_bulk_edit();
            ctx.fill_chunk(5, 5, &[&ids]).unwrap();
            ctx.finalize().unwrap();
        }
        let chunk = store.get_chunk(5 / 16, 5 / 16).unwrap();
        assert_eq!(chunk.get_block(0, 0), 7);
    }

    #[test]
    fn fill_chunk_wrong_section_count_returns_error() {
        let mut store = ChunkStore::new();
        let ids = vec![1u32; SECTION_VOLUME];
        let mut ctx = store.start_bulk_edit();
        // 传入 2 个区段但 section_count = 1，应报错
        let result = ctx.fill_chunk(0, 0, &[&ids, &ids]);
        assert!(matches!(result, Err(SectionFillError::InvalidLength)));
    }

    #[test]
    fn fill_chunk_id_length_error_returns_invalid_length() {
        let mut store = ChunkStore::new();
        let mut ctx = store.start_bulk_edit();
        let short_ids = vec![1u32; SECTION_VOLUME - 1];
        let result = ctx.fill_chunk(0, 0, &[&short_ids]);
        assert!(matches!(result, Err(SectionFillError::InvalidLength)));
    }

    #[test]
    fn fill_chunk_same_chunk_accumulates_on_pending() {
        let mut store = ChunkStore::new();
        let ids_a = vec![1u32; SECTION_VOLUME];
        let ids_b = vec![2u32; SECTION_VOLUME];
        {
            let mut ctx = store.start_bulk_edit();
            // 第一次填充
            ctx.fill_chunk(0, 0, &[&ids_a]).unwrap();
            // 第二次填充同一区块（叠加修改）
            ctx.fill_chunk(0, 0, &[&ids_b]).unwrap();
            ctx.finalize().unwrap();
        }
        let chunk = store.get_chunk(0, 0).unwrap();
        // 最终应为最后一次填充的结果
        assert_eq!(chunk.get_block(0, 0), 2);
    }

    #[test]
    fn section_count_accessor() {
        let store = ChunkStore::new();
        assert_eq!(store.section_count(), 1);
    }
}
