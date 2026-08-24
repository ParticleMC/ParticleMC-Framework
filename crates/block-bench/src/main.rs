// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
// 方块写入压力测试（优化版）：使用 BulkEditContext 批量填充。
//
// 规模：31×31 网格 = 961 区块，每区块 1 个区段 × 4096 格（y=0..15），
// 共 3,936,256 次写入。
//
// 与原 block-bench 对比：本版本使用 fill_blocks / BulkEditContext，
// 跳过 per-write repack + recompute_light + LRU，验证批量填充性能。

use particlemc_framework_core::instance::chunk::SectionFillError;
use particlemc_framework_core::instance::chunk_store::ChunkStore;
use std::time::Instant;

/// 确定性伪随机块 ID 生成。
fn pseudo_block_id(x: i32, y: i32, z: i32) -> u32 {
    let h = ((x as u64).wrapping_mul(73)
        ^ (y as u64).wrapping_mul(131)
        ^ (z as u64).wrapping_mul(179))
        .rotate_left(17);
    (h % 200) as u32 + 1
}

fn main() -> Result<(), SectionFillError> {
    const GRID_SIZE: i32 = 31;
    const OFFSET: i32 = GRID_SIZE / 2;
    const SECTIONS_PER_CHUNK: usize = 1;
    const Y_MIN: i32 = 0;
    const Y_MAX: i32 = 15;

    let chunks_per_side = GRID_SIZE;
    let total_chunks = (chunks_per_side * chunks_per_side) as usize;
    let blocks_per_chunk = SECTIONS_PER_CHUNK * particlemc_framework_core::instance::chunk::SECTION_VOLUME;
    let total_blocks = total_chunks * blocks_per_chunk;

    println!("=== Block Fill/Read/Clear Stress Test (BulkEdit) ===");
    println!("  Grid: {chunks_per_side}×{chunks_per_side} = {total_chunks} chunks");
    println!("  Sections/chunk: {SECTIONS_PER_CHUNK}, blocks/section: {}", particlemc_framework_core::instance::chunk::SECTION_VOLUME);
    println!("  Total blocks:  {}", total_blocks as u64);
    println!("  Y range: [{Y_MIN}, {Y_MAX}]");
    println!();

    let mut store = ChunkStore::new();
    for cx in 0_i32..chunks_per_side {
        for cz in 0_i32..chunks_per_side {
            let wx = cx - OFFSET;
            let wz = cz - OFFSET;
            store.load_chunk(particlemc_framework_core::instance::chunk::Chunk::new(wx, wz, SECTIONS_PER_CHUNK));
        }
    }
    println!("Pre-loaded {} chunks. Ready.\n", total_chunks);

    // ── 阶段 1：批量填充 ─────────────────────────────────────────────────
    let t_start = Instant::now();
    {
        let mut ctx = store.start_bulk_edit();
        for cx in 0_i32..chunks_per_side {
            for cz in 0_i32..chunks_per_side {
                let wx = cx - OFFSET;
                let wz = cz - OFFSET;
                let mut sections_data: Vec<Vec<u32>> = Vec::with_capacity(SECTIONS_PER_CHUNK);
                for _ in 0..SECTIONS_PER_CHUNK {
                    let mut ids = vec![0u32; particlemc_framework_core::instance::chunk::SECTION_VOLUME];
                    for y in Y_MIN..=Y_MAX {
                        for lz in 0_i32..16 {
                            for lx in 0_i32..16 {
                                let idx = ((y - Y_MIN) as usize) * 256 + (lz as usize) * 16 + (lx as usize);
                                ids[idx] = pseudo_block_id(wx * 16 + lx, y, wz * 16 + lz);
                            }
                        }
                    }
                    sections_data.push(ids);
                }
                let refs: Vec<&[u32]> = sections_data.iter().map(|v| v.as_slice()).collect();
                ctx.fill_chunk(wx, wz, &refs)?;
            }
        }
        ctx.finalize()?;
    }
    let fill_dur = t_start.elapsed();
    println!(
        "[Phase 1 - Fill]    {:>12}  blocks/s  |  {:>10.3} ms",
        total_blocks as u64 * 1_000_000_000 / fill_dur.as_nanos() as u64,
        fill_dur.as_secs_f64() * 1000.0,
    );

    // ── 阶段 2：读取验证 ─────────────────────────────────────────────────
    let t_read = Instant::now();
    let mut zero_count = 0usize;
    let mut non_zero_min = u32::MAX;
    let mut non_zero_max = 0u32;
    for cx in 0_i32..chunks_per_side {
        for cz in 0_i32..chunks_per_side {
            let wx = cx - OFFSET;
            let wz = cz - OFFSET;
            for y in Y_MIN..=Y_MAX {
                for lz in 0_i32..16 {
                    for lx in 0_i32..16 {
                        let id = store.get_block_id_world(wx * 16 + lx, y, wz * 16 + lz);
                        if id == 0 {
                            zero_count += 1;
                        } else {
                            if id < non_zero_min { non_zero_min = id; }
                            if id > non_zero_max { non_zero_max = id; }
                        }
                    }
                }
            }
        }
    }
    let read_dur = t_read.elapsed();
    println!(
        "[Phase 2 - Read]    {:>12}  blocks/s  |  {:>10.3} ms",
        total_blocks as u64 * 1_000_000_000 / read_dur.as_nanos() as u64,
        read_dur.as_secs_f64() * 1000.0,
    );
    println!("  zero blocks:     {}, non-zero range: [{}, {}]",
        zero_count, non_zero_min, non_zero_max);

    // ── 阶段 3：批量清空 ─────────────────────────────────────────────────
    let t_clear = Instant::now();
    {
        let mut ctx = store.start_bulk_edit();
        for cx in 0_i32..chunks_per_side {
            for cz in 0_i32..chunks_per_side {
                let wx = cx - OFFSET;
                let wz = cz - OFFSET;
                let mut sections_data: Vec<Vec<u32>> = Vec::with_capacity(SECTIONS_PER_CHUNK);
                for _ in 0..SECTIONS_PER_CHUNK {
                    sections_data.push(vec![0u32; particlemc_framework_core::instance::chunk::SECTION_VOLUME]);
                }
                let refs: Vec<&[u32]> = sections_data.iter().map(|v| v.as_slice()).collect();
                ctx.fill_chunk(wx, wz, &refs)?;
            }
        }
        ctx.finalize()?;
    }
    let clear_dur = t_clear.elapsed();
    println!(
        "[Phase 3 - Clear]   {:>12}  blocks/s  |  {:>10.3} ms",
        total_blocks as u64 * 1_000_000_000 / clear_dur.as_nanos() as u64,
        clear_dur.as_secs_f64() * 1000.0,
    );

    let total = fill_dur + read_dur + clear_dur;
    println!("\n=== Summary ===");
    println!("  Fill rate:     {:.2} M blocks/s",
        total_blocks as f64 / fill_dur.as_secs_f64() / 1_000_000.0);
    println!("  Read rate:     {:.2} M blocks/s",
        total_blocks as f64 / read_dur.as_secs_f64() / 1_000_000.0);
    println!("  Clear rate:    {:.2} M blocks/s",
        total_blocks as f64 / clear_dur.as_secs_f64() / 1_000_000.0);
    println!("  Total time:    {:.3} ms", total.as_secs_f64() * 1000.0);
    println!("  Avg throughput:{:.2} M blocks/s",
        (total_blocks * 3) as f64 / total.as_secs_f64() / 1_000_000.0);

    Ok(())
}
