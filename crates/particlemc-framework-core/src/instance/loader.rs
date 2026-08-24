//! 区块持久化抽象：`ChunkLoader` trait + 内存实现。
//!
//! 语义对齐 Minestom Java 的 `ChunkLoader`（`load` 返回整块数据且不再应用
//! 生成器；`save` 持久化整块），但**不复制 Java 实现**。当前只提供内存快照
//! 实现 [`MemoryChunkLoader`] 与 trait 本身，用于世界容器的加载/保存管线。
//!
//! **范围说明**：真实 MCA 区域文件读写不在本任务范围（另行变更，README 已注明）；
//! 后续可新增 `AnvilChunkLoader` 之类的实现替换内存快照。
//!
//! 变更标识符：`complete-missing-subsystems`（R8 ChunkGenerator 接口 + 内存快照存档）。
//! 见 `.specs/complete-missing-subsystems/spec.md`。

use std::collections::HashMap;

use super::chunk::Chunk;

/// 区块持久化器：负责区块的加载与保存。
///
/// 实现必须 `Send + Sync`，以便被世界容器持有；`load` 返回整块数据（调用方
/// 不再对其应用生成器），`save` 持久化整块。
pub trait ChunkLoader: Send + Sync {
    /// 加载指定坐标的区块；不存在时返回 `None`。
    fn load(&mut self, x: i32, z: i32) -> Option<Chunk>;
    /// 保存一个区块（按区块坐标覆盖旧数据）。
    fn save(&mut self, chunk: &Chunk);
    /// 是否已保存过指定坐标的区块。
    fn contains(&self, x: i32, z: i32) -> bool;
    /// 返回全部已保存区块的坐标列表，供世界容器的 `load_all` 枚举。
    ///
    /// 默认返回空列表；仅当实现能枚举自身持有的区块时才需要覆盖。
    fn keys(&self) -> Vec<(i32, i32)> {
        Vec::new()
    }
}

/// 内存快照加载器：以 `(x, z)` 为键把区块克隆保存在进程内存中。
///
/// `load` 返回克隆、`save` 存入克隆，调用方持有的区块与内部快照互不影响。
#[derive(Debug, Default)]
pub struct MemoryChunkLoader {
    /// 坐标 → 区块快照。
    chunks: HashMap<(i32, i32), Chunk>,
}

impl MemoryChunkLoader {
    /// 构造一个空的内存快照加载器。
    pub fn new() -> Self {
        Self::default()
    }

    /// 已保存区块数量。
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }
}

impl ChunkLoader for MemoryChunkLoader {
    fn load(&mut self, x: i32, z: i32) -> Option<Chunk> {
        self.chunks.get(&(x, z)).cloned()
    }

    fn save(&mut self, chunk: &Chunk) {
        self.chunks.insert((chunk.x, chunk.z), chunk.clone());
    }

    fn contains(&self, x: i32, z: i32) -> bool {
        self.chunks.contains_key(&(x, z))
    }

    fn keys(&self) -> Vec<(i32, i32)> {
        self.chunks.keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_loader_roundtrip() {
        let mut loader = MemoryChunkLoader::new();
        let mut chunk = Chunk::new(3, -2, 2);
        assert!(chunk.set_block(1, 100, 7));
        loader.save(&chunk);
        assert!(loader.contains(3, -2));
        let loaded = loader.load(3, -2).expect("chunk 已保存应可加载");
        assert_eq!(loaded, chunk);
    }

    #[test]
    fn load_missing_returns_none() {
        let mut loader = MemoryChunkLoader::new();
        assert!(loader.load(0, 0).is_none());
        assert!(!loader.contains(0, 0));
        assert_eq!(loader.chunk_count(), 0);
    }

    #[test]
    fn save_replaces_existing_chunk() {
        let mut loader = MemoryChunkLoader::new();
        let mut first = Chunk::new(0, 0, 1);
        assert!(first.set_block(0, 1, 5));
        loader.save(&first);
        let mut second = Chunk::new(0, 0, 1);
        assert!(second.set_block(0, 2, 9));
        loader.save(&second);
        let loaded = loader.load(0, 0).expect("chunk 已保存应可加载");
        assert_eq!(loaded.get_block(0, 2), 9);
        assert_eq!(loader.chunk_count(), 1);
    }

    #[test]
    fn keys_enumerate_saved_chunks() {
        let mut loader = MemoryChunkLoader::new();
        loader.save(&Chunk::new(0, 0, 1));
        loader.save(&Chunk::new(-1, 5, 1));
        let mut keys = loader.keys();
        keys.sort();
        assert_eq!(keys, vec![(-1, 5), (0, 0)]);
    }
}
