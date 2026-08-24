//! 实体坐标组件。
//!
//! 以 `f64` 存储世界坐标（x/y/z），`f32` 存储朝向（yaw/pitch），契合 Minecraft
//! 的坐标精度需求。`Position` 为 `Copy`，便于在系统中按值读取。

use crate::prelude::Component;

/// 实体在 world 中的坐标与朝向。
#[derive(Default, Component, Debug, Clone, Copy, PartialEq)]
#[component(storage = "sparse")]
pub struct Position {
    /// 世界 X 坐标。
    pub x: f64,
    /// 世界 Y 坐标。
    pub y: f64,
    /// 世界 Z 坐标。
    pub z: f64,
    /// 偏航角（弧度）。
    pub yaw: f32,
    /// 俯仰角（弧度）。
    pub pitch: f32,
}

impl Position {
    /// 以零朝向构造一个坐标点。
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Self {
            x,
            y,
            z,
            yaw: 0.0,
            pitch: 0.0,
        }
    }

    /// 以完整朝向构造一个坐标点。
    pub fn with_rotation(x: f64, y: f64, z: f64, yaw: f32, pitch: f32) -> Self {
        Self {
            x,
            y,
            z,
            yaw,
            pitch,
        }
    }

    /// 返回 X 坐标。
    pub fn x(&self) -> f64 {
        self.x
    }

    /// 返回 Y 坐标。
    pub fn y(&self) -> f64 {
        self.y
    }

    /// 返回 Z 坐标。
    pub fn z(&self) -> f64 {
        self.z
    }

    /// 返回偏航角。
    pub fn yaw(&self) -> f32 {
        self.yaw
    }

    /// 返回俯仰角。
    pub fn pitch(&self) -> f32 {
        self.pitch
    }
}
