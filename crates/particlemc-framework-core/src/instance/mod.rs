// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 实例层：Minestom 的世界模型骨架。
//!
//! 对应 Minestom 的 `Instance` / `Chunk` / `Section`，骨架阶段提供方块读写的最小
//! 真实逻辑与占位容器，供后续真实世界生成与 IO 接入。

pub mod anvil;
pub mod chunk;
pub mod chunk_serializer;
pub mod chunk_store;
pub mod fluid;
pub mod generator;
pub mod heightmap;
pub mod instance_container;
pub mod light;
pub mod light_engine;
pub mod loader;

pub use anvil::AnvilChunkLoader;
pub use chunk::{Chunk, SECTION_VOLUME, Section, SectionFillError};
pub use chunk_serializer::{SerializedChunk, serialize_chunk, serialize_section};
pub use chunk_store::{ChunkStore, BulkEditContext};
pub use fluid::{Fluid, fluid_from_block};
#[allow(deprecated)]
pub use generator::NoopChunkGenerator;
pub use generator::{ChunkGenerator, GeneratedChunk, NoiseChunkGenerator, generated_to_chunk};
pub use heightmap::{Heightmap, build_motion_blocking};
pub use instance_container::InstanceContainer;
pub use light::LightSystem;
pub use light_engine::{LightBoundaryDir, LightEngine, SectionLightBoundary};
pub use loader::{ChunkLoader, MemoryChunkLoader};
