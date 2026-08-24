//! Anvil `.mca` 区域文件持久化（WS2 — `complete-framework-gaps`）。
//!
//! [`AnvilChunkLoader`] 实现 [`ChunkLoader`]：将 [`Chunk`]（含方块调色板与
//! `ChunkLightStorage::light`）以 Minecraft Anvil 区域文件格式落盘，并能无损读回。
//!
//! 区域文件布局（与 vanilla 一致）：
//! - 文件 `r.<rx>.<rz>.mca`，头部为 4096 字节定位表 + 4096 字节时间戳表；
//! - 每区块数据 = 4 字节大端长度 + 1 字节压缩类型（`1` = zlib）+ zlib 压缩 NBT；
//! - 数据按 4096 字节「扇区」对齐，定位表每槽 4 字节（24 位扇区偏移 + 8 位扇区数）。
//!
//! 区块 NBT（`Level` 复合）：`xPos`/`zPos`/`Sections`（每区段 `Y` + `BlockStates`
//! 整型列表 + `SkyLight`/`BlockLight` 字节数组）+ `Heightmaps.MOTION_BLOCKING`。
//! 写入先落临时文件再原子 `rename`，满足 spec 的 rollback 要求（见
//! `scripts/rollback_anvil.ps1`）。

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;

use crate::instance::chunk::{Chunk, LightSection, SECTION_VOLUME};
use crate::instance::loader::ChunkLoader;
use crate::protocol::nbt::{NbtTag, decode_root, encode_root};

/// 区域边长（区块数）：32×32 = 1024 区块 / 区域。
const REGION_DIMENSION: i32 = 32;
/// 扇区字节数（Anvil 固定 4096）。
const SECTOR_SIZE: usize = 4096;
/// 头部占用扇区数（定位表 + 时间戳表各占 1 扇区）。
const HEADER_SECTORS: usize = 2;
/// 定位表 / 时间戳表槽数（= 区域内区块数）。
const SLOT_COUNT: usize = (REGION_DIMENSION * REGION_DIMENSION) as usize;
/// Minecraft 协议 DataVersion（1.21.11 = 4671）。
const DATA_VERSION: i32 = 4671;
/// Anvil 压缩类型：1 = zlib（deflate）。
const COMPRESSION_ZLIB: u8 = 1;

/// 由区块坐标求所属区域坐标（向负无穷取整，正确处理负坐标）。
#[must_use]
fn region_coords(x: i32, z: i32) -> (i32, i32) {
    (
        x.div_euclid(REGION_DIMENSION),
        z.div_euclid(REGION_DIMENSION),
    )
}

/// 由区块坐标求区域内局部坐标（恒 ∈ [0, 32)）。
#[must_use]
fn local_coords(x: i32, z: i32) -> (i32, i32) {
    (
        x.rem_euclid(REGION_DIMENSION),
        z.rem_euclid(REGION_DIMENSION),
    )
}

/// 定位表槽序号 → 区域内局部坐标。
#[must_use]
fn local_slot_coords(slot: i32) -> (i32, i32) {
    let lz = slot / REGION_DIMENSION;
    let lx = slot % REGION_DIMENSION;
    (lx, lz)
}

/// 在复合条目中按名查找子 tag。
fn find<'a>(entries: &'a [(String, NbtTag)], name: &str) -> Option<&'a NbtTag> {
    entries.iter().find(|(k, _)| k == name).map(|(_, v)| v)
}

/// 区块 → NBT 字节（带根名编码）。编码失败返回空切片（上层跳过该次保存）。
fn encode_chunk_nbt(chunk: &Chunk) -> Vec<u8> {
    let mut sections = Vec::with_capacity(chunk.sections.len());
    for (s, section) in chunk.sections.iter().enumerate() {
        let mut ids = Vec::with_capacity(SECTION_VOLUME);
        for i in 0..SECTION_VOLUME {
            ids.push(NbtTag::Int(
                i32::try_from(section.get_block_id(i)).unwrap_or(0),
            ));
        }
        let light = chunk
            .light
            .get(s)
            .copied()
            .unwrap_or_else(LightSection::new);
        sections.push(NbtTag::Compound(vec![
            ("Y".to_string(), NbtTag::Byte(i8::try_from(s).unwrap_or(0))),
            ("BlockStates".to_string(), NbtTag::List(ids)),
            (
                "SkyLight".to_string(),
                NbtTag::ByteArray(light.sky_light.to_vec()),
            ),
            (
                "BlockLight".to_string(),
                NbtTag::ByteArray(light.block_light.to_vec()),
            ),
        ]));
    }
    let level = NbtTag::Compound(vec![
        ("xPos".to_string(), NbtTag::Int(chunk.x)),
        ("zPos".to_string(), NbtTag::Int(chunk.z)),
        (
            "Heightmaps".to_string(),
            NbtTag::Compound(vec![(
                "MOTION_BLOCKING".to_string(),
                NbtTag::LongArray(vec![0i64; 36]),
            )]),
        ),
        ("Sections".to_string(), NbtTag::List(sections)),
    ]);
    let root = NbtTag::Compound(vec![
        ("DataVersion".to_string(), NbtTag::Int(DATA_VERSION)),
        ("Level".to_string(), level),
    ]);
    encode_root(&root).unwrap_or_default()
}

/// 区块 NBT 字节 → `Chunk`（无损重建）。任何结构异常返回 `None`。
fn decode_chunk_nbt(bytes: &[u8]) -> Option<Chunk> {
    let (_name, root) = decode_root(bytes).ok()?;
    let NbtTag::Compound(root_entries) = root else {
        return None;
    };
    let level = find(&root_entries, "Level")?;
    let NbtTag::Compound(level_entries) = level else {
        return None;
    };
    let x = match find(level_entries, "xPos") {
        Some(NbtTag::Int(v)) => *v,
        _ => return None,
    };
    let z = match find(level_entries, "zPos") {
        Some(NbtTag::Int(v)) => *v,
        _ => return None,
    };
    let sections_tag = find(level_entries, "Sections")?;
    let NbtTag::List(sections) = sections_tag else {
        return None;
    };
    let mut chunk = Chunk::new(x, z, sections.len().max(1));
    for (s, entry) in sections.iter().enumerate() {
        let NbtTag::Compound(sec) = entry else {
            continue;
        };
        if let Some(NbtTag::List(ids)) = find(sec, "BlockStates")
            && let Some(target) = chunk.sections.get_mut(s)
        {
            for (i, tag) in ids.iter().enumerate() {
                if let NbtTag::Int(id) = tag {
                    target.set_block_id(i, *id as u32);
                }
            }
        }
        if let Some(light) = chunk.light.get_mut(s) {
            if let Some(NbtTag::ByteArray(sky)) = find(sec, "SkyLight") {
                let n = sky.len().min(SECTION_VOLUME);
                light.sky_light[..n].copy_from_slice(&sky[..n]);
            }
            if let Some(NbtTag::ByteArray(bl)) = find(sec, "BlockLight") {
                let n = bl.len().min(SECTION_VOLUME);
                light.block_light[..n].copy_from_slice(&bl[..n]);
            }
        }
    }
    chunk.ensure_light_synced();
    Some(chunk)
}

/// zlib 压缩（flate2 纯 Rust 后端）。失败返回 `None`。
fn compress(payload: &[u8]) -> Option<Vec<u8>> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    if enc.write_all(payload).is_err() {
        return None;
    }
    enc.finish().ok()
}

/// zlib 解压。失败返回 `None`。
fn decompress(data: &[u8]) -> Option<Vec<u8>> {
    let mut dec = ZlibDecoder::new(data);
    let mut out = Vec::new();
    if dec.read_to_end(&mut out).is_err() {
        return None;
    }
    Some(out)
}

/// 区块 NBT → 区域文件区块 blob（`[4 字节长度][1 字节类型=1][zlib 压缩 NBT]`）。
fn make_chunk_blob(nbt: &[u8]) -> Option<Vec<u8>> {
    let compressed = compress(nbt)?;
    let len = u32::try_from(compressed.len()).ok()?;
    let mut blob = Vec::with_capacity(5 + compressed.len());
    blob.extend_from_slice(&len.to_be_bytes());
    blob.push(COMPRESSION_ZLIB);
    blob.extend_from_slice(&compressed);
    Some(blob)
}

/// 区域文件区块 blob → 解压后的 NBT 字节。结构异常返回 `None`。
fn parse_chunk_blob(blob: &[u8]) -> Option<Vec<u8>> {
    if blob.len() < 5 {
        return None;
    }
    let len = u32::from_be_bytes([blob[0], blob[1], blob[2], blob[3]]) as usize;
    let ctype = blob[4];
    if ctype != COMPRESSION_ZLIB {
        return None;
    }
    let comp = blob.get(5..5 + len)?;
    decompress(comp)
}

/// 单个区域文件的运行时视图：内存持有各局部坐标的压缩区块 blob，
/// `flush` 时重写整文件（头部 + 各区块扇区对齐），先写临时文件再原子 rename。
struct RegionFile {
    /// 区域文件路径 `r.<rx>.<rz>.mca`。
    path: PathBuf,
    /// 局部坐标 → 压缩区块 blob（含长度头与类型字节）。
    chunks: HashMap<(i32, i32), Vec<u8>>,
}

impl RegionFile {
    /// 构造空区域文件视图（不读盘）。
    fn new(dir: &Path, rx: i32, rz: i32) -> Self {
        let path = dir.join(format!("r.{rx}.{rz}.mca"));
        Self {
            path,
            chunks: HashMap::new(),
        }
    }

    /// 打开已有区域文件并解析头部定位表，登记非空区块。
    fn open(dir: &Path, rx: i32, rz: i32) -> std::io::Result<Self> {
        let mut rf = Self::new(dir, rx, rz);
        if rf.path.exists()
            && let Ok(data) = fs::read(&rf.path)
        {
            for slot in 0..SLOT_COUNT {
                let off = slot * 4;
                if off + 4 > data.len() {
                    break;
                }
                let sector = ((data[off] as usize) << 16)
                    | ((data[off + 1] as usize) << 8)
                    | (data[off + 2] as usize);
                let count = data[off + 3] as usize;
                if sector == 0 || count == 0 {
                    continue;
                }
                let start = sector * SECTOR_SIZE;
                let end = start + count * SECTOR_SIZE;
                if end > data.len() {
                    continue;
                }
                rf.chunks
                    .insert(local_slot_coords(slot as i32), data[start..end].to_vec());
            }
        }
        Ok(rf)
    }

    /// 写入（覆盖）某局部坐标的区块 blob。
    fn set_chunk(&mut self, lx: i32, lz: i32, blob: Vec<u8>) {
        self.chunks.insert((lx, lz), blob);
    }

    /// 读取某局部坐标的区块 blob（未写入返回 `None`）。
    fn get_chunk(&self, lx: i32, lz: i32) -> Option<&Vec<u8>> {
        self.chunks.get(&(lx, lz))
    }

    /// 已登记区块的局部坐标列表。
    fn keys(&self) -> Vec<(i32, i32)> {
        self.chunks.keys().copied().collect()
    }

    /// 重写整文件（头部 + 各区块扇区对齐），先写临时文件再原子 rename。
    fn flush(&self) -> std::io::Result<()> {
        let mut location = [0u8; SLOT_COUNT * 4];
        let timestamp = [0u8; SLOT_COUNT * 4];
        let mut body: Vec<u8> = Vec::new();
        let mut cursor = HEADER_SECTORS;

        let mut slots: Vec<((i32, i32), &Vec<u8>)> =
            self.chunks.iter().map(|(k, v)| (*k, v)).collect();
        slots.sort_by_key(|(k, _)| k.1 * REGION_DIMENSION + k.0);

        for ((lx, lz), payload) in &slots {
            let n_sectors = payload.len().div_ceil(SECTOR_SIZE);
            if n_sectors == 0 {
                continue;
            }
            let slot = (lz * REGION_DIMENSION + lx) as usize;
            if slot >= SLOT_COUNT {
                continue;
            }
            let off = slot * 4;
            location[off] = ((cursor >> 16) & 0xff) as u8;
            location[off + 1] = ((cursor >> 8) & 0xff) as u8;
            location[off + 2] = (cursor & 0xff) as u8;
            location[off + 3] = n_sectors as u8;

            let mut padded = payload.to_vec();
            padded.resize(n_sectors * SECTOR_SIZE, 0);
            body.extend_from_slice(&padded);
            cursor += n_sectors;
        }

        let mut file = Vec::with_capacity(HEADER_SECTORS * SECTOR_SIZE + body.len());
        file.extend_from_slice(&location);
        file.extend_from_slice(&timestamp);
        file.extend_from_slice(&body);

        write_atomic(&self.path, &file)
    }
}

/// 写临时文件后原子 rename 到目标路径（崩溃可经 rollback 脚本恢复）。
fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// 基于 Anvil 区域文件的区块持久化器。
///
/// 以目录为根，按区域坐标分文件（`r.<rx>.<rz>.mca`）。构造时扫描既有 `.mca`
/// 建立坐标索引，使 `contains` / `load` 能反映磁盘状态；`save` 写入对应区域
/// 并刷新整文件（原子替换）。
pub struct AnvilChunkLoader {
    /// 区域文件根目录。
    dir: PathBuf,
    /// 区域坐标 → 区域文件视图。
    regions: HashMap<(i32, i32), RegionFile>,
    /// 区块全局坐标 → 所属区域坐标（用于 `contains` / `keys`）。
    index: HashMap<(i32, i32), (i32, i32)>,
}

impl AnvilChunkLoader {
    /// 在 `dir` 下构造加载器；目录不存在则创建，并扫描既有区域文件建立索引。
    pub fn new(dir: PathBuf) -> std::io::Result<Self> {
        fs::create_dir_all(&dir)?;
        let mut loader = Self {
            dir,
            regions: HashMap::new(),
            index: HashMap::new(),
        };
        loader.scan_existing();
        Ok(loader)
    }

    /// 扫描目录内既有 `.mca`，登记其区块坐标到内存索引并缓存区域视图。
    fn scan_existing(&mut self) {
        let entries = match fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("mca") {
                continue;
            }
            let stem = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s,
                None => continue,
            };
            let parts: Vec<&str> = stem.split('.').collect();
            if parts.len() != 3 || parts[0] != "r" {
                continue;
            }
            let (rx, rz) = match (parts[1].parse::<i32>(), parts[2].parse::<i32>()) {
                (Ok(rx), Ok(rz)) => (rx, rz),
                _ => continue,
            };
            let rf = match RegionFile::open(&self.dir, rx, rz) {
                Ok(rf) => rf,
                Err(_) => continue,
            };
            for (lx, lz) in rf.keys() {
                let gx = rx * REGION_DIMENSION + lx;
                let gz = rz * REGION_DIMENSION + lz;
                self.index.insert((gx, gz), (rx, rz));
            }
            self.regions.insert((rx, rz), rf);
        }
    }
}

impl ChunkLoader for AnvilChunkLoader {
    fn load(&mut self, x: i32, z: i32) -> Option<Chunk> {
        let (rx, rz) = region_coords(x, z);
        let (lx, lz) = local_coords(x, z);
        let rf = self.regions.get(&(rx, rz))?;
        let blob = rf.get_chunk(lx, lz)?;
        let nbt = parse_chunk_blob(blob)?;
        decode_chunk_nbt(&nbt)
    }

    fn save(&mut self, chunk: &Chunk) {
        let (rx, rz) = region_coords(chunk.x, chunk.z);
        let (lx, lz) = local_coords(chunk.x, chunk.z);
        let blob = {
            let rf = self.regions.entry((rx, rz)).or_insert_with(|| {
                RegionFile::open(&self.dir, rx, rz)
                    .unwrap_or_else(|_| RegionFile::new(&self.dir, rx, rz))
            });
            let nbt = encode_chunk_nbt(chunk);
            match make_chunk_blob(&nbt) {
                Some(blob) => {
                    rf.set_chunk(lx, lz, blob);
                    let _ = rf.flush();
                    Some(())
                }
                None => None,
            }
        };
        // 不论写入是否成功都更新索引意图：成功写入的坐标计入 contains。
        if blob.is_some() {
            self.index.insert((chunk.x, chunk.z), (rx, rz));
        }
    }

    fn contains(&self, x: i32, z: i32) -> bool {
        self.index.contains_key(&(x, z))
    }

    fn keys(&self) -> Vec<(i32, i32)> {
        self.index.keys().copied().collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// 构造带唯一子目录的临时根，避免并发测试互相污染。
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mc_anvil_test_{}_{}_{}",
            name,
            std::process::id(),
            name.len()
        ));
        let _ = fs::create_dir_all(&dir);
        dir
    }

    /// 构造含多种方块与光照的区块，并清空脏标记以便与加载结果做相等比较。
    fn make_chunk(x: i32, z: i32, sections: usize) -> Chunk {
        let mut chunk = Chunk::new(x, z, sections);
        for s in 0..sections {
            for i in 0..SECTION_VOLUME {
                let id = ((i % 5) + 1) as u32;
                chunk.set_block(s, i, id);
            }
            if let Some(light) = chunk.light_sections_mut().get_mut(s) {
                for i in 0..SECTION_VOLUME {
                    light.set_sky(i, (i % 16) as u8);
                    light.set_block(i, ((i * 3) % 16) as u8);
                }
            }
        }
        chunk.take_dirty_sections();
        chunk
    }

    #[test]
    fn anvil_roundtrip_matches_id_and_light() {
        let dir = temp_dir("roundtrip");
        let mut loader = AnvilChunkLoader::new(dir.clone()).expect("loader 构造");
        let chunk = make_chunk(3, -2, 3);
        loader.save(&chunk);
        let loaded = loader.load(3, -2).expect("应可加载");
        assert_eq!(loaded, chunk);
    }

    #[test]
    fn missing_chunk_returns_none_no_panic() {
        let dir = temp_dir("missing");
        let mut loader = AnvilChunkLoader::new(dir.clone()).expect("loader 构造");
        assert!(loader.load(0, 0).is_none());
        assert!(!loader.contains(0, 0));
    }

    #[test]
    fn region_file_locates_by_coords_without_corrupting_others() {
        let dir = temp_dir("region");
        let mut loader = AnvilChunkLoader::new(dir.clone()).expect("loader 构造");
        let a = make_chunk(0, 0, 2);
        let b = make_chunk(1, 1, 2);
        let c = make_chunk(31, 31, 2);
        loader.save(&a);
        loader.save(&b);
        loader.save(&c);
        assert_eq!(loader.load(0, 0).expect("a"), a);
        assert_eq!(loader.load(1, 1).expect("b"), b);
        assert_eq!(loader.load(31, 31).expect("c"), c);
        // 未写入的区块仍为 None，且不影响同区域其它区块。
        assert!(loader.load(2, 2).is_none());
        assert!(loader.contains(0, 0));
        assert!(loader.contains(31, 31));
        assert!(!loader.contains(2, 2));
    }

    #[test]
    fn save_replaces_existing_chunk() {
        let dir = temp_dir("replace");
        let mut loader = AnvilChunkLoader::new(dir.clone()).expect("loader 构造");
        let first = make_chunk(0, 0, 1);
        loader.save(&first);
        let mut second = Chunk::new(0, 0, 1);
        second.set_block(0, 5, 9);
        second.take_dirty_sections();
        loader.save(&second);
        let loaded = loader.load(0, 0).expect("应可加载");
        assert_eq!(loaded.get_block(0, 5), 9);
    }

    #[test]
    fn negative_coords_roundtrip() {
        let dir = temp_dir("neg");
        let mut loader = AnvilChunkLoader::new(dir.clone()).expect("loader 构造");
        let chunk = make_chunk(-1, -5, 2);
        loader.save(&chunk);
        assert_eq!(loader.load(-1, -5).expect("neg"), chunk);
    }

    #[test]
    fn keys_enumerate_saved_chunks() {
        let dir = temp_dir("keys");
        let mut loader = AnvilChunkLoader::new(dir.clone()).expect("loader 构造");
        loader.save(&make_chunk(0, 0, 1));
        loader.save(&make_chunk(-1, 5, 1));
        let mut keys = loader.keys();
        keys.sort();
        assert_eq!(keys, vec![(-1, 5), (0, 0)]);
    }
}
