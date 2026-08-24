//! Message 事件系统：tick 内生命周期的事件通道。
//!
//! 变更标识符：`implement-custom-ecs`
//!
//! 对齐 自研 ECS 的 Message 语义（R6.3 / IC-8）：`MessageWriter` 写入 →
//! 后续系统经 `MessageReader` 读取 → tick 末由 Schedule 对全部注册 inbox
//! 调 `clear` 清空。事件缓冲预分配（默认 64，满时几何翻倍），tick 内正常
//! 路径零堆分配（R3.4）。
//!
//! 消费方为 T7（`Schedule::add_message::<T>()` 注册 inbox 并持有到 tick 末
//! 统一清空）与 T10/T11（particlemc-framework-core 系统签名中的 `MessageWriter` /
//! `MessageReader` 参数）。

use crate::util::next_power_of_two;

/// 消息标记契约：可跨线程发送、可共享引用、生命周期 `'static`。
///
/// 与 旧 ECS 方案 语义一致。由 `#[derive(Message)]`（T2 宏）生成显式空实现；本
/// crate **不提供 blanket impl**——宏展开可能发生在外部 crate，若存在 blanket
/// impl 会与之冲突。这是与 `Resource`（有 blanket impl）的关键差异。
pub trait Message: Send + Sync + 'static {}

/// 预分配事件缓冲（tick 内生命周期）。
///
/// `buffer` 为追加式存储，`head` 标记已读位置：`read`/`iter` 只遍历
/// `buffer[head..]`，不推进游标（同一 tick 内多次读取看到同一批消息）；
/// `clear`（tick 末）丢弃全部元素并保留容量，供下一 tick 零分配复用。
/// 容量按几何翻倍增长（[`next_power_of_two`]），正常路径不触发分配。
pub struct MessageInbox<T: Message> {
    /// 追加式缓冲：保留容量跨 tick 复用。
    buffer: Vec<T>,
    /// 已读位置：`read` 从该下标遍历至缓冲末尾。
    head: usize,
}

impl<T: Message> MessageInbox<T> {
    /// 默认容量 64 的空 inbox（首次扩容前零分配）。
    pub fn new() -> Self {
        Self::with_capacity(64)
    }

    /// 以指定容量预分配的空 inbox。
    pub fn with_capacity(cap: usize) -> Self {
        MessageInbox {
            buffer: Vec::with_capacity(cap),
            head: 0,
        }
    }

    /// 追加一条消息；容量不足时按几何翻倍扩容（[`next_power_of_two`]）。
    pub fn write(&mut self, msg: T) {
        if self.buffer.len() == self.buffer.capacity() {
            let target = next_power_of_two(self.buffer.capacity().saturating_mul(2));
            self.buffer.reserve(target - self.buffer.capacity());
        }
        self.buffer.push(msg);
    }

    /// 从已读位置到缓冲末尾的迭代器（不推进已读游标）。
    pub fn read(&self) -> impl Iterator<Item = &T> + '_ {
        self.buffer.iter().skip(self.head)
    }

    /// [`read`] 的别名。
    pub fn iter(&self) -> impl Iterator<Item = &T> + '_ {
        self.read()
    }

    /// 清空全部消息并返回已清数量；容量保留（下一 tick 零分配复用）。
    ///
    /// 本 tick 内已读与未读消息一并丢弃——"tick 末清空"语义，由 Schedule
    /// （T7）在每个 tick 结束时对所有注册 inbox 调用。
    pub fn clear(&mut self) -> usize {
        let cleared = self.buffer.len();
        self.buffer.clear();
        self.head = 0;
        cleared
    }

    /// 未读消息数量（缓冲总长减去已读位置）。
    pub fn len(&self) -> usize {
        self.buffer.len() - self.head
    }

    /// 是否无未读消息。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T: Message> Default for MessageInbox<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// 写入端系统参数：独占持有 inbox 的可变引用。
///
/// T7 的 `SystemParam` 实现经 `MessageWriter(&mut inbox)` 构造注入系统；
/// 同一 tick 内多个 writer 依调度顺序依次追加。
pub struct MessageWriter<'w, T: Message>(pub(crate) &'w mut MessageInbox<T>);

impl<T: Message> MessageWriter<'_, T> {
    /// 追加一条消息到本 tick 的事件缓冲。
    pub fn write(&mut self, msg: T) {
        self.0.write(msg);
    }

    /// [`write`] 的别名（与 旧 ECS 方案 `send` 语义对应）。
    pub fn send(&mut self, msg: T) {
        self.write(msg);
    }
}

/// 读取端系统参数：共享持有 inbox 的只读引用。
///
/// T7 的 `SystemParam` 实现经 `MessageReader(&inbox)` 构造注入系统；
/// 同一 tick 内可多次读取同一批消息（不消耗）。
pub struct MessageReader<'w, T: Message>(pub(crate) &'w MessageInbox<T>);

impl<T: Message> MessageReader<'_, T> {
    /// 本 tick 内可读的全部消息（从已读位置到缓冲末尾）。
    pub fn read(&self) -> impl Iterator<Item = &T> + '_ {
        self.0.read()
    }

    /// [`read`] 的别名。
    pub fn iter(&self) -> impl Iterator<Item = &T> + '_ {
        self.0.iter()
    }

    /// 本 tick 内未读消息数量。
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// 本 tick 内是否无未读消息。
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试消息：带序号的整数包装。
    #[derive(Debug, PartialEq, Eq)]
    struct Num(u32);

    impl Message for Num {}

    /// 第二个消息类型：用于泛型隔离断言。
    struct Tag;

    impl Message for Tag {}

    #[test]
    fn write_then_read_sees_all_in_order() {
        let mut inbox = MessageInbox::new();
        inbox.write(Num(1));
        inbox.write(Num(2));
        inbox.write(Num(3));
        let got: Vec<u32> = inbox.read().map(|n| n.0).collect();
        assert_eq!(got, vec![1, 2, 3]);
    }

    #[test]
    fn clear_drops_all_and_keeps_capacity() {
        let mut inbox = MessageInbox::with_capacity(8);
        inbox.write(Num(1));
        inbox.write(Num(2));
        let cap_before = inbox.buffer.capacity();
        let cleared = inbox.clear();
        assert_eq!(cleared, 2);
        // 容量保留：clear 后 capacity 不变（下一 tick 零分配复用）
        assert_eq!(inbox.buffer.capacity(), cap_before);
        assert_eq!(inbox.read().count(), 0);
        assert!(inbox.is_empty());
        assert_eq!(inbox.len(), 0);
    }

    #[test]
    fn writes_within_capacity_do_not_reallocate() {
        let mut inbox = MessageInbox::with_capacity(64);
        let cap_before = inbox.buffer.capacity();
        for i in 0..64 {
            inbox.write(Num(i));
        }
        // 容量充足时 push 不触发分配：capacity 前后不变
        assert_eq!(inbox.buffer.capacity(), cap_before);
        assert_eq!(inbox.len(), 64);
    }

    #[test]
    fn capacity_grows_geometrically_when_full() {
        let mut inbox = MessageInbox::with_capacity(2);
        let cap_before = inbox.buffer.capacity();
        for i in 0..2 {
            inbox.write(Num(i));
        }
        inbox.write(Num(2)); // 触发翻倍扩容
        assert!(inbox.buffer.capacity() > cap_before);
        assert_eq!(inbox.len(), 3);
        let got: Vec<u32> = inbox.iter().map(|n| n.0).collect();
        assert_eq!(got, vec![0, 1, 2]);
    }

    #[test]
    fn writer_and_reader_wrappers_delegate() {
        let mut inbox = MessageInbox::new();
        {
            let mut writer = MessageWriter(&mut inbox);
            writer.write(Num(10));
            writer.send(Num(20));
        }
        let reader = MessageReader(&inbox);
        assert_eq!(reader.len(), 2);
        let got: Vec<u32> = reader.read().map(|n| n.0).collect();
        assert_eq!(got, vec![10, 20]);
        // iter 为 read 的别名，路径独立覆盖
        let via_iter: Vec<u32> = reader.iter().map(|n| n.0).collect();
        assert_eq!(via_iter, vec![10, 20]);
    }

    #[test]
    fn different_message_types_are_isolated() {
        let mut nums = MessageInbox::<Num>::new();
        let mut tags = MessageInbox::<Tag>::new();
        nums.write(Num(1));
        assert_eq!(nums.len(), 1);
        assert!(tags.is_empty());
        assert_eq!(tags.read().count(), 0);
        tags.write(Tag);
        assert_eq!(tags.len(), 1);
        assert_eq!(nums.len(), 1);
    }

    #[test]
    fn empty_inbox_reads_as_empty() {
        let mut inbox = MessageInbox::<Num>::new();
        assert_eq!(inbox.read().count(), 0);
        assert!(inbox.is_empty());
        assert_eq!(inbox.len(), 0);
        assert_eq!(inbox.clear(), 0);
    }

    #[test]
    fn read_is_non_consuming_within_tick() {
        let mut inbox = MessageInbox::new();
        inbox.write(Num(5));
        // read 不推进游标：同一 tick 内重复读取看到相同消息
        assert_eq!(inbox.read().count(), 1);
        assert_eq!(inbox.read().count(), 1);
        assert_eq!(inbox.len(), 1);
    }

    #[test]
    fn buffer_reused_across_ticks() {
        let mut inbox = MessageInbox::with_capacity(64);
        let cap_before = inbox.buffer.capacity();
        for i in 0..32 {
            inbox.write(Num(i));
        }
        inbox.clear();
        // 下一 tick 复用既有容量，不重新分配
        for i in 0..32 {
            inbox.write(Num(i + 100));
        }
        assert_eq!(inbox.buffer.capacity(), cap_before);
        let got: Vec<u32> = inbox.iter().map(|n| n.0).collect();
        assert_eq!(got, (100..132).collect::<Vec<u32>>());
    }

    #[test]
    fn zero_capacity_grows_on_first_write() {
        let mut inbox = MessageInbox::with_capacity(0);
        assert_eq!(inbox.buffer.capacity(), 0);
        inbox.write(Num(1));
        assert!(inbox.buffer.capacity() > 0);
        assert_eq!(inbox.len(), 1);
    }
}
