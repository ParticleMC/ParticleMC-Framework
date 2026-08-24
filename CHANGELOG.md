<!-- Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
SPDX-License-Identifier: GPL-3.0-or-later -->
# Changelog

所有 notable 变更将记录在此文件中。格式遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)，
版本遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### BREAKING — 命名空间重命名（rename-minestom-to-particlemc-framework）

**原因**：框架品牌名称从 "Minestom" 统一变更为 "ParticleMCFramework"，使命名空间与项目目录名保持一致。

**变更内容**：

- **Crate 名称重命名**：
  - `minestom-ecs` → `particlemc-framework-ecs`
  - `minestom-ecs-macros` → `particlemc-framework-ecs-macros`
  - `minestom-core` → `particlemc-framework-core`
  - `minestom-server` → `particlemc-framework-server`

- **Rust 模块路径**：
  - `minestom_ecs::` → `particlemc_framework_ecs::`
  - `minestom_ecs_macros::` → `particlemc_framework_ecs_macros::`
  - `minestom_core::` → `particlemc_framework_core::`
  - `minestom_server::` → `particlemc_framework_server::`

- **环境变量**：
  - `MINESTOM_BIND_ADDR` → `PARTICLE_MCFRAMEWORK_BIND_ADDR`
  - `MINESTOM_VELOCITY_SECRET` → `PARTICLE_MCFRAMEWORK_VELOCITY_SECRET`
  - `MINESTOM_COMPRESSION_THRESHOLD` → `PARTICLE_MCFRAMEWORK_COMPRESSION_THRESHOLD`

- **WASM 初始化符号**：
  - `minestom_init` → `particlemc_framework_init`
  - `minestom_tick` → `particlemc_framework_tick`

- **资源数据注释**：`# Source: net/minestom/data/...` → `# Source: net/particlemc-framework/data/...`

- **CI 工作流**：路径与包名同步更新

**迁移指南**：
所有引用旧 crate 名称或模块路径的外部代码需要同步更新为新名称。
