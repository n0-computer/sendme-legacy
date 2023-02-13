//! Generic utilities to track progress of data transfers.
//!
//! This is not especially specific to sendme but can be helpful together with it.  The
//! [`ProgressEmitter`] has a [`ProgressEmitter::wrap_async_read`] method which can make it
//! easy to track process of transfers.
//!
//! However based on your environment there might also be better choices for this, e.g. very
//! similar and more advanced functionality is available in the `indicatif` crate for
//! terminal applications.

use std::pin::Pin;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::Arc;
use std::task::Poll;

use tokio::io::{self, AsyncRead};
use tokio::sync::broadcast;

/// A generic progress event emitter.
///
/// It is created with a total value to reach and at which increments progress should be
/// emitted.  E.g. when downloading a file of any size but you want percentage increments
/// you would create `ProgressEmitter::new(file_size_in_bytes, 100)` and
/// [`ProgressEmitter::subscribe`] will yield numbers `1..100` only.
///
/// Progress is made by calling [`ProgressEmitter::inc`], which can be implicitly done by
/// [`ProgressEmitter::wrap_async_read`].
#[derive(Debug, Clone)]
pub struct ProgressEmitter {
    inner: Arc<InnerProgressEmitter>,
}

impl ProgressEmitter {
    /// Creates a new emitter.
    ///
    /// The emitter expects to see *total* being added via [`ProgressEmitter::inc`] and will
    /// emit *steps* updates.
    pub fn new(total: u64, steps: u16) -> Self {
        let (tx, _rx) = broadcast::channel(16);
        Self {
            inner: Arc::new(InnerProgressEmitter {
                total: AtomicU64::new(total),
                count: AtomicU64::new(0),
                steps,
                last_step: AtomicU16::new(0u16),
                tx,
            }),
        }
    }

    /// Sets a new total in case you did not now the total up front.
    pub fn set_total(&self, value: u64) {
        self.inner.set_total(value)
    }

    /// Returns a receiver that gets incremental values.
    ///
    /// The values yielded depend on *steps* passed to [`ProgressEmitter::new`]: it will go
    /// from `1..steps`.
    pub fn subscribe(&self) -> broadcast::Receiver<u16> {
        self.inner.subscribe()
    }

    /// Increments the progress by *amount*.
    pub fn inc(&self, amount: u64) {
        self.inner.inc(amount);
    }

    /// Wraps an [`AsyncRead`] which implicitly calls [`ProgressEmitter::inc`].
    pub fn wrap_async_read<R: AsyncRead + Unpin>(&self, read: R) -> ProgressAsyncReader<R> {
        ProgressAsyncReader {
            emitter: self.clone(),
            inner: read,
        }
    }
}

/// The actual implementation.
///
/// This exists so it can be Arc'd into [`ProgressEmitter`] and we can easily have multiple
/// `Send + Sync` copies of it.  This is used by the
/// [`ProgressEmitter::ProgressAsyncReader`] to update the progress without intertwining
/// lifetimes.
#[derive(Debug)]
struct InnerProgressEmitter {
    total: AtomicU64,
    count: AtomicU64,
    steps: u16,
    last_step: AtomicU16,
    tx: broadcast::Sender<u16>,
}

impl InnerProgressEmitter {
    fn inc(&self, amount: u64) {
        let prev_count = self.count.fetch_add(amount, Ordering::Relaxed);
        let count = prev_count + amount;
        let total = self.total.load(Ordering::Relaxed);
        let step = (std::cmp::min(count, total) * u64::from(self.steps) / total) as u16;
        let last_step = self.last_step.swap(step, Ordering::Relaxed);
        if step > last_step {
            self.tx.send(step).ok();
        }
    }

    fn set_total(&self, value: u64) {
        self.total.store(value, Ordering::Relaxed);
    }

    fn subscribe(&self) -> broadcast::Receiver<u16> {
        self.tx.subscribe()
    }
}

/// A wrapper around [`AsyncRead`] which increments a [`ProgressEmitter`].
///
/// This can be used just like the underlying [`AsyncRead`] but increments progress for each
/// byte read.  Create this using [`ProgressEmitter::wrap_async_read`].
#[derive(Debug)]
pub struct ProgressAsyncReader<R: AsyncRead + Unpin> {
    emitter: ProgressEmitter,
    inner: R,
}

impl<R> AsyncRead for ProgressAsyncReader<R>
where
    R: AsyncRead + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let prev_len = buf.filled().len() as u64;
        match Pin::new(&mut self.inner).poll_read(cx, buf) {
            Poll::Ready(val) => {
                let new_len = buf.filled().len() as u64;
                self.emitter.inc(new_len - prev_len);
                Poll::Ready(val)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
