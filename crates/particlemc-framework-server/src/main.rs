// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! # particlemc-framework-server
//!
//! ParticleMCFramework（Rust 重写版）服务器二进制入口。构造并装配自研 `App`、启动真实
//! TCP 监听，并进入 20Hz 游戏主循环（见 [`particlemc_framework_server::run`]）。

use std::net::SocketAddr;

use particlemc_framework_server::run;

/// 服务器二进制入口。
fn main() {
    let addr: SocketAddr = std::env::var("PARTICLE_MCFRAMEWORK_BIND_ADDR")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], 25565)));
    print_banner();
    if let Err(e) = run(addr) {
        eprintln!("服务器异常退出：{e}");
        std::process::exit(1);
    }
}

/// 打印 ParticleMCFramework 服务器启动横幅。
fn print_banner() {
    println!("========================================");
    println!(" ParticleMCFramework (Rust) - 可用框架层");
    println!(" 自研 particlemc-framework-ecs 内核 | 20Hz tick (20 TPS)");
    println!(" 真实 TCP 监听 + 三层发包模型已启用");
    println!("========================================");
}
