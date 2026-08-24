// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 实体速度组件。
//!
//! 以 `f64` 存储三轴速度（方块/秒）。`Velocity` 为 `Copy`，便于在物理系统中按值运算。

use crate::prelude::Component;

/// 实体速度（方块/秒）。
#[derive(Default, Component, Debug, Clone, Copy, PartialEq)]
#[component(storage = "sparse")]
pub struct Velocity {
    /// X 轴速度。
    pub x: f64,
    /// Y 轴速度。
    pub y: f64,
    /// Z 轴速度。
    pub z: f64,
}

impl Velocity {
    /// 以三轴速度构造。
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    /// 零速度。
    pub fn zero() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }
    }

    /// 返回 X 轴速度。
    pub fn x(&self) -> f64 {
        self.x
    }

    /// 返回 Y 轴速度。
    pub fn y(&self) -> f64 {
        self.y
    }

    /// 返回 Z 轴速度。
    pub fn z(&self) -> f64 {
        self.z
    }

    /// 与另一速度逐轴相加，得到新的合成速度。
    pub fn add(&self, other: &Velocity) -> Velocity {
        Velocity {
            x: self.x + other.x,
            y: self.y + other.y,
            z: self.z + other.z,
        }
    }
}
