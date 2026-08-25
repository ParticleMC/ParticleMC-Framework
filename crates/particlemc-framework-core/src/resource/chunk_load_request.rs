// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 区块加载请求队列（G3-T1）。
//!
//! 持有待异步加载的区块坐标，供 [`super::chunk_boundary`] 投递、
//! 异步 worker 消费。FIFO 语义，无重复检测（由调用方保证）。

use std::collections::VecDeque;

/// 单条区块加载请求。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkLoadRequest {
    /// 世界 id（由调用方在投递时指定，当前仅支持默认实例）。
    pub world_id: u32,
    /// 区块 X 坐标（区块空间，非世界坐标）。
    pub chunk_x: i32,
    /// 区块 Z 坐标（区块空间，非世界坐标）。
    pub chunk_z: i32,
}

/// 区块加载请求队列（线程安全，供异步 worker 与 tick 系统共享）。
///
/// 采用 `Mutex<VecDeque>` 封装，push 与 pop 均为 O(1)。
#[derive(Default)]
pub struct ChunkLoadQueue {
    inner: std::sync::Mutex<VecDeque<ChunkLoadRequest>>,
}

impl ChunkLoadQueue {
    /// 构造空队列。
    pub fn new() -> Self {
        Self {
            inner: std::sync::Mutex::new(VecDeque::new()),
        }
    }

    /// 将一条加载请求入队。
    pub fn push(&self, req: ChunkLoadRequest) {
        let mut q = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        q.push_back(req);
    }

    /// 出队一条请求；队列为空返回 `None`。
    pub fn pop(&self) -> Option<ChunkLoadRequest> {
        let mut q = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        q.pop_front()
    }

    /// 队列是否为空。
    pub fn is_empty(&self) -> bool {
        let q = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        q.is_empty()
    }

    /// 当前队列长度。
    pub fn len(&self) -> usize {
        let q = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        q.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_queue_pops_none() {
        let q = ChunkLoadQueue::new();
        assert!(q.pop().is_none());
        assert!(q.is_empty());
    }

    #[test]
    fn push_and_pop_fifo() {
        let q = ChunkLoadQueue::new();
        q.push(ChunkLoadRequest {
            world_id: 0,
            chunk_x: 1,
            chunk_z: 2,
        });
        q.push(ChunkLoadRequest {
            world_id: 0,
            chunk_x: 3,
            chunk_z: 4,
        });
        assert_eq!(q.pop(), Some(ChunkLoadRequest { world_id: 0, chunk_x: 1, chunk_z: 2 }));
        assert_eq!(q.pop(), Some(ChunkLoadRequest { world_id: 0, chunk_x: 3, chunk_z: 4 }));
        assert!(q.pop().is_none());
    }

    #[test]
    fn len_reflects_pushes() {
        let q = ChunkLoadQueue::new();
        assert_eq!(q.len(), 0);
        q.push(ChunkLoadRequest { world_id: 0, chunk_x: 0, chunk_z: 0 });
        assert_eq!(q.len(), 1);
        q.pop();
        assert_eq!(q.len(), 0);
    }
}
