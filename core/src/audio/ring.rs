//! 有界环形缓冲：播放侧 200–500ms 抗抖动（`docs/开发方案.md` 4.3）。
//!
//! 设计要点：
//! - **有界**：满了就覆盖最旧的数据（丢旧比丢新对实时语音更友好 —— 用户已经等不到的话，
//!   不如把最新的说出来），并用 [`SampleRing::overwritten`] 报告累计丢弃量，便于监控。
//! - **可即时清空**：[`SampleRing::clear`] 配合不变量 I4（打断时队列秒清）。
//! - 只存 `f32` 单声道样本；调用方负责采样率与声道（采集侧已下混成 mono）。
//!
//! 缓冲是**非线程安全**的 `Vec` + 读写游标；跨线程使用请在外层包 `Mutex`
//! （播放侧的实际用法见 [`super::playback`]）。

/// 固定容量的有界环形缓冲。
///
/// `capacity` 是样本数上限（例如 24kHz × 300ms = 7200）。
#[derive(Debug, Clone)]
pub struct SampleRing {
    buf: Vec<f32>,
    /// 下一次写入的位置（逻辑下标，0..capacity）。
    write: usize,
    /// 当前有效样本数（<= capacity）。
    len: usize,
    /// 累计因溢出被覆盖的样本数。
    overwritten: u64,
}

impl SampleRing {
    /// 建一个最多存 `capacity` 个样本的环。`capacity == 0` 时所有写入都被丢弃。
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: vec![0.0; capacity],
            write: 0,
            len: 0,
            overwritten: 0,
        }
    }

    /// 按「毫秒 × 采样率」建环，方便直接写 `SampleRing::with_duration(24_000, 300)`。
    pub fn with_duration(sample_rate: u32, ms: u32) -> Self {
        let cap = ((u64::from(sample_rate) * u64::from(ms)) / 1000) as usize;
        Self::new(cap)
    }

    /// 容量（样本数）。
    #[inline]
    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    /// 当前已存样本数。
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// 是否为空。
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// 是否已满。
    #[inline]
    pub fn is_full(&self) -> bool {
        self.len == self.buf.len()
    }

    /// 空闲可写空间（样本数）。
    #[inline]
    pub fn free(&self) -> usize {
        self.buf.len() - self.len
    }

    /// 累计被新数据覆盖掉的旧样本数。
    #[inline]
    pub fn overwritten(&self) -> u64 {
        self.overwritten
    }

    /// 清空（不改变容量，不重置 `overwritten` 统计）。
    pub fn clear(&mut self) {
        self.write = 0;
        self.len = 0;
    }

    /// 清空并连统计一起归零。
    pub fn reset(&mut self) {
        self.clear();
        self.overwritten = 0;
    }

    /// 压入一个样本。环满时覆盖最旧的样本。
    pub fn push(&mut self, sample: f32) {
        let cap = self.buf.len();
        if cap == 0 {
            self.overwritten = self.overwritten.saturating_add(1);
            return;
        }
        self.buf[self.write] = sample;
        self.write = (self.write + 1) % cap;
        if self.len < cap {
            self.len += 1;
        } else {
            // 满环：写游标追上读端，等于丢了一个最旧样本
            self.overwritten = self.overwritten.saturating_add(1);
        }
    }

    /// 压入一批样本，返回实际写入数（无界环时等于 `samples.len()`）。
    pub fn extend_from_slice(&mut self, samples: &[f32]) -> usize {
        for &s in samples {
            self.push(s);
        }
        samples.len()
    }

    /// 弹出一个最旧样本。
    pub fn pop(&mut self) -> Option<f32> {
        let cap = self.buf.len();
        if self.len == 0 || cap == 0 {
            return None;
        }
        // 最旧元素位于 write - len（模 cap）
        let read = (self.write + cap - self.len) % cap;
        let v = self.buf[read];
        self.len -= 1;
        Some(v)
    }

    /// 取出最多 `out.len()` 个最旧样本填进 `out`，返回实际填充数。
    pub fn pop_into(&mut self, out: &mut [f32]) -> usize {
        let mut n = 0;
        while n < out.len() {
            match self.pop() {
                Some(v) => {
                    out[n] = v;
                    n += 1;
                }
                None => break,
            }
        }
        n
    }

    /// 取出全部样本（按时间顺序）。
    pub fn drain_all(&mut self) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.len);
        while let Some(v) = self.pop() {
            out.push(v);
        }
        out
    }

    /// 只读窥视，不消费。返回按时间顺序排列的最多 `max` 个样本。
    pub fn peek(&self, max: usize) -> Vec<f32> {
        let cap = self.buf.len();
        if cap == 0 {
            return Vec::new();
        }
        let n = self.len.min(max);
        let start = (self.write + cap - self.len) % cap;
        (0..n).map(|i| self.buf[(start + i) % cap]).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 新环是空的() {
        let mut r = SampleRing::new(8);
        assert_eq!(r.capacity(), 8);
        assert_eq!(r.len(), 0);
        assert!(r.is_empty());
        assert!(!r.is_full());
        assert_eq!(r.free(), 8);
        assert_eq!(r.pop(), None);
    }

    #[test]
    fn 先进先出顺序正确() {
        let mut r = SampleRing::new(4);
        r.extend_from_slice(&[1.0, 2.0, 3.0]);
        assert_eq!(r.len(), 3);
        assert_eq!(r.pop(), Some(1.0));
        assert_eq!(r.pop(), Some(2.0));
        r.push(4.0);
        r.push(5.0);
        // 此刻内容是 [3,4,5]
        assert_eq!(r.peek(10), vec![3.0, 4.0, 5.0]);
        assert_eq!(r.drain_all(), vec![3.0, 4.0, 5.0]);
        assert!(r.is_empty());
    }

    #[test]
    fn 满环覆盖最旧并计数() {
        let mut r = SampleRing::new(3);
        r.extend_from_slice(&[1.0, 2.0, 3.0]);
        assert!(r.is_full());
        assert_eq!(r.overwritten(), 0);

        r.push(4.0); // 覆盖 1.0
        assert_eq!(r.peek(3), vec![2.0, 3.0, 4.0]);
        assert_eq!(r.overwritten(), 1);

        r.extend_from_slice(&[5.0, 6.0]); // 覆盖 2.0、3.0
        assert_eq!(r.peek(3), vec![4.0, 5.0, 6.0]);
        assert_eq!(r.overwritten(), 3);
        assert_eq!(r.len(), 3);
        assert_eq!(r.free(), 0);
    }

    #[test]
    fn 清空后从头开始() {
        let mut r = SampleRing::new(3);
        r.extend_from_slice(&[1.0, 2.0, 3.0]);
        r.clear();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
        // 统计保留（用于日志观察丢弃量），数据是新的
        r.extend_from_slice(&[9.0, 8.0]);
        assert_eq!(r.drain_all(), vec![9.0, 8.0]);

        r.extend_from_slice(&[7.0]);
        r.reset();
        assert_eq!(r.overwritten(), 0);
    }

    #[test]
    fn 打断场景_插话后立即清空() {
        // 不变量 I4 的前置：播放侧塞了 300ms 数据，用户插话 → clear 后必须立刻 0
        let mut r = SampleRing::with_duration(24_000, 300);
        assert_eq!(r.capacity(), 7_200);
        r.extend_from_slice(&vec![0.25f32; 7_000]);
        assert_eq!(r.len(), 7_000);
        r.clear();
        assert_eq!(r.len(), 0, "clear 后队列必须为 0");
        assert!(r.pop().is_none());
    }

    #[test]
    fn pop_into_部分填充() {
        let mut r = SampleRing::new(8);
        r.extend_from_slice(&[1.0, 2.0, 3.0]);
        let mut out = [0.0f32; 5];
        assert_eq!(r.pop_into(&mut out), 3);
        assert_eq!(&out[..3], &[1.0, 2.0, 3.0]);
        assert_eq!(r.len(), 0);

        r.extend_from_slice(&[1.0, 2.0, 3.0, 4.0]);
        let mut out = [0.0f32; 2];
        assert_eq!(r.pop_into(&mut out), 2);
        assert_eq!(out, [1.0, 2.0]);
        assert_eq!(r.len(), 2);

        // 空输入安全
        let mut empty: [f32; 0] = [];
        assert_eq!(r.pop_into(&mut empty), 0);
    }

    #[test]
    fn 零容量环不panic() {
        let mut r = SampleRing::new(0);
        assert_eq!(r.capacity(), 0);
        r.push(1.0);
        assert!(r.is_empty());
        assert_eq!(r.overwritten(), 1);
        assert_eq!(r.pop(), None);
        assert!(r.peek(4).is_empty());
    }

    #[test]
    fn 环形绕回正确() {
        // 反复 push/pop 触发多次绕回，验证读写游标不漂移
        let mut r = SampleRing::new(5);
        let mut next_in = 0.0f32;
        let mut next_out = 0.0f32;
        for _ in 0..1000 {
            r.push(next_in);
            next_in += 1.0;
            if r.len() == 3 {
                let got = r.pop().unwrap();
                assert_eq!(got, next_out);
                next_out += 1.0;
            }
        }
        assert_eq!(r.overwritten(), 0);
        while let Some(v) = r.pop() {
            assert_eq!(v, next_out);
            next_out += 1.0;
        }
        assert_eq!(next_in, 1000.0);
    }

    #[test]
    fn with_duration_换算() {
        assert_eq!(SampleRing::with_duration(24_000, 200).capacity(), 4_800);
        assert_eq!(SampleRing::with_duration(24_000, 500).capacity(), 12_000);
        assert_eq!(SampleRing::with_duration(16_000, 20).capacity(), 320);
        assert_eq!(SampleRing::with_duration(16_000, 0).capacity(), 0);
    }
}
