// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Streaming `ResponseWriter` helpers for actix-web (or any) HTTP host.
//!
//! `webui-handler` writes through a push-based [`ResponseWriter`] trait —
//! every `Raw` fragment, attribute, signal value, route element open/close,
//! CSS preload `<link>`, and template assignment is a separate `write()`
//! call (~hundreds per render). The default host pattern collects them all
//! into a `String`, then serves the whole HTML body in one shot — which
//! delays first-byte until the entire render finishes and forces the
//! browser to wait for everything before parsing.
//!
//! The helpers here let a host **flush bytes to the network as soon as
//! they're written**:
//!
//! * [`StreamingWriter`] — coalesces small writes into ~4 KB chunks and
//!   pushes them through a **bounded** [`tokio::sync::mpsc::Sender`]. The
//!   bound (`DEFAULT_CHANNEL_CAPACITY = 4` chunks ≈ 16 KB) provides
//!   backpressure: when a slow client cannot keep up, the producer parks
//!   on the channel until the receiver drains, instead of queuing the
//!   entire response in memory. A configurable flush deadline (via
//!   [`with_flush_timeout`](StreamingWriter::with_flush_timeout)) caps
//!   the maximum time a producer thread can be parked, bounding the
//!   slow-loris DoS surface to `timeout × concurrent_renders`. When the
//!   receiver is dropped (client disconnect) or the deadline elapses,
//!   `write` returns a typed error so the handler aborts instead of
//!   doing wasted CPU work.
//!
//! * [`ChunkPool`] — lock-free shared pool of chunk buffers. Used via
//!   [`StreamingWriter::new_pooled`] to recycle the per-flush `Vec<u8>`
//!   across requests, eliminating per-flush heap allocation in
//!   steady-state high-RPS workloads.
//!
//! Hot-path allocation profile:
//!
//! * `StreamingWriter::new()` (unpooled): one `Vec::reserve` per ~4 KB
//!   flush (the previous buffer is moved zero-copy into [`bytes::Bytes`]
//!   when `len < cap`; when `len == cap`, `Bytes::from(Vec)` is still a
//!   move via `into_boxed_slice`). Plus one small `Box<Shared>` for the
//!   refcount metadata.
//! * `StreamingWriter::new_pooled()`: zero per-flush heap allocation in
//!   steady state — chunk buffers come from the pool and return on
//!   `Bytes` drop. Single atomic CAS per acquire/release.
//!
//! # Per-render HTML injection
//!
//! Hosts that need to splice HTML at the structural `</head>` or `</body>`
//! boundaries (image preload `<link>` tags, dev livereload `<script>`,
//! CSP nonce reflections, analytics, etc.) should use
//! [`RenderOptions::with_head_inject`] / [`RenderOptions::with_body_inject`]
//! on the handler side — NOT a writer-level scanner. The parser already
//! synthesises `head_end` / `body_end` signal fragments at the exact
//! structural boundaries; the handler emits the inject HTML there with
//! zero scan cost and no risk of mis-firing on `</body>` literals
//! appearing inside HTML comments, `<iframe srcdoc>`, or inline scripts.
//!
//! [`RenderOptions::with_head_inject`]: webui_handler::RenderOptions::with_head_inject
//! [`RenderOptions::with_body_inject`]: webui_handler::RenderOptions::with_body_inject

use bytes::Bytes;
use crossbeam_queue::ArrayQueue;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::Sender;
use webui_handler::{HandlerError, ResponseWriter, Result};

// ── ChunkPool ──────────────────────────────────────────────────────

/// Lock-free shared pool of `Vec<u8>` buffers used to recycle chunk
/// allocations across `StreamingWriter` instances.
///
/// Backed by a [`crossbeam_queue::ArrayQueue`] (MPMC, lock-free, fixed
/// capacity). Acquiring a buffer is a single atomic CAS; releasing is
/// the same. When the pool is empty, `acquire` allocates a fresh
/// `Vec<u8>`. When the pool is full, `release` drops the buffer.
///
/// # Lifetime model
///
/// A buffer leaves the pool on `acquire`, gets handed to
/// [`bytes::Bytes::from_owner`] wrapped in a [`PooledChunk`] owner,
/// and is released back to the pool when **the last `Bytes` reference
/// is dropped** — typically after the HTTP framework has flushed the
/// chunk to the wire. Because `Bytes` may be dropped on any thread
/// (the actix worker that wrote the chunk to the socket, not the
/// `spawn_blocking` worker that produced it), the pool MUST be
/// thread-safe — `ArrayQueue` is.
///
/// # Sizing
///
/// `max_pool` should match the expected concurrent in-flight chunk
/// count: `concurrent_renders × channel_capacity` in the worst case.
/// For the production setup (4-chunk channels, ~100 concurrent
/// renders), `max_pool = 512` covers the working set; surplus buffers
/// are dropped when full so memory cannot grow unboundedly.
///
/// `chunk_size` should match `StreamingWriter::CHUNK_TARGET +
/// BUF_HEADROOM`. When acquiring, the writer always grows the buffer
/// if the pool returned a smaller one (host code that mixes pool
/// sizes pays a one-time grow per buffer).
///
/// # Cost
///
/// * `acquire`: 1 atomic CAS (~10 ns on x86) + an `unwrap_or_else`
///   that allocates only on miss.
/// * `release`: 1 atomic CAS + drop-on-overflow.
/// * Pool storage: `max_pool * size_of::<AtomicCell<Vec<u8>>>` =
///   ~32 bytes per slot, i.e. 512 slots = 16 KiB pool overhead.
///
/// # Example
///
/// ```ignore
/// use std::sync::Arc;
/// use webui::streaming::{ChunkPool, StreamingWriter};
///
/// // Construct ONE pool at server startup:
/// let pool = Arc::new(ChunkPool::new(512, StreamingWriter::CHUNK_TARGET));
///
/// // Each request:
/// let (tx, rx) = tokio::sync::mpsc::channel(StreamingWriter::DEFAULT_CHANNEL_CAPACITY);
/// let writer = StreamingWriter::new_pooled(tx, Arc::clone(&pool));
/// ```
pub struct ChunkPool {
    queue: ArrayQueue<Vec<u8>>,
    chunk_size: usize,
}

impl ChunkPool {
    /// Create a new shared chunk pool. Wrap in `Arc` and share across
    /// all `StreamingWriter` instances that should recycle their
    /// chunk buffers.
    ///
    /// `max_pool` is the maximum number of buffers held idle at once.
    /// Surplus buffers are dropped (returned to the allocator) — this
    /// caps total pool memory at `max_pool × chunk_size`.
    ///
    /// `chunk_size` is the initial capacity used when allocating a
    /// fresh buffer on a pool miss. Pre-sizing avoids a Vec-grow on
    /// the hot path.
    #[must_use]
    pub fn new(max_pool: usize, chunk_size: usize) -> Self {
        Self {
            // ArrayQueue requires capacity > 0.
            queue: ArrayQueue::new(max_pool.max(1)),
            chunk_size,
        }
    }

    /// Acquire a buffer from the pool, or allocate a fresh one if the
    /// pool is empty. The returned `Vec` is empty (`len == 0`); its
    /// capacity is at least `chunk_size` (may be larger if a previous
    /// caller grew it).
    ///
    /// Trusts that callers (only [`PooledChunk::drop`] in this crate)
    /// have already cleared the buffer before release. In debug builds
    /// we assert the invariant; release builds skip the check to keep
    /// `acquire` to a single CAS + capacity check.
    fn acquire(&self) -> Vec<u8> {
        match self.queue.pop() {
            Some(mut buf) => {
                debug_assert!(
                    buf.is_empty(),
                    "ChunkPool invariant violation: pool returned non-empty buffer"
                );
                if buf.capacity() < self.chunk_size {
                    buf.reserve(self.chunk_size - buf.capacity());
                }
                buf
            }
            None => Vec::with_capacity(self.chunk_size),
        }
    }

    /// Release a buffer back to the pool. The buffer is `clear()`-ed
    /// here (cheap — sets `len` to 0, no deallocation), so `acquire`
    /// can trust the invariant and skip a defensive clear on the hot
    /// path. Drops the buffer if the pool is full.
    fn release(&self, mut buf: Vec<u8>) {
        buf.clear();
        // ArrayQueue::push returns Err with the value if full; we
        // simply drop in that case.
        let _ = self.queue.push(buf);
    }

    /// Number of buffers currently idle in the pool. Snapshot-only;
    /// useful for diagnostic metrics.
    #[must_use]
    pub fn idle_count(&self) -> usize {
        self.queue.len()
    }

    /// Maximum buffers the pool can hold idle.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.queue.capacity()
    }
}

/// Owner type given to [`bytes::Bytes::from_owner`] so the chunk
/// buffer returns to the pool when the last `Bytes` reference drops.
///
/// `AsRef<[u8]>` is the only contract `Bytes::from_owner` requires;
/// the data pointer it captures stays valid as long as `self` is
/// alive (the `Bytes` keeps `self` alive via its internal owner box).
struct PooledChunk {
    /// `Option` so we can `take()` the `Vec` in `Drop` and return
    /// it to the pool — Drop receives `&mut self`, so we can't move
    /// out of the field directly. Using `Option` keeps the impl
    /// safe (no `ManuallyDrop` / `unsafe`) at the cost of one
    /// 8-byte tag per chunk-in-flight; negligible vs the chunk size.
    buf: Option<Vec<u8>>,
    pool: Arc<ChunkPool>,
}

impl PooledChunk {
    #[inline]
    fn new(buf: Vec<u8>, pool: Arc<ChunkPool>) -> Self {
        Self {
            buf: Some(buf),
            pool,
        }
    }
}

impl AsRef<[u8]> for PooledChunk {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        // INVARIANT: `buf` is always `Some` until `Drop`. Any `None`
        // observation here would be a use-after-take bug. We use a
        // safe match instead of unwrap to comply with the workspace's
        // `disallowed-methods` lint.
        match self.buf.as_deref() {
            Some(slice) => slice,
            // Should never happen — Drop is the only `take` site, and
            // `Bytes::from_owner` doesn't expose `&mut`. Returning
            // `&[]` here is defensive: the wire would just see a
            // truncated chunk rather than a panic.
            None => &[],
        }
    }
}

impl Drop for PooledChunk {
    fn drop(&mut self) {
        if let Some(buf) = self.buf.take() {
            // `release` clears the buffer; we don't double-clear here.
            self.pool.release(buf);
        }
    }
}

// ── StreamingWriter ────────────────────────────────────────────────

/// Streaming `ResponseWriter` backed by a **bounded** tokio mpsc channel
/// of [`Bytes`].
///
/// Coalesces small writes into ~4 KB chunks before flushing. The
/// underlying channel has a small bound
/// ([`DEFAULT_CHANNEL_CAPACITY`](Self::DEFAULT_CHANNEL_CAPACITY)) so a
/// slow consumer naturally backpressures the producer — the render
/// thread parks instead of queuing the entire response in memory.
///
/// A flush deadline ([`with_flush_timeout`](Self::with_flush_timeout))
/// caps the maximum time the producer thread will park per flush,
/// bounding the slow-loris DoS surface. When the receiver is dropped
/// (typically client disconnect) or the deadline is exceeded, subsequent
/// [`ResponseWriter::write`] calls return a typed error
/// ([`HandlerError::ClientDisconnected`] / [`HandlerError::StreamTimeout`])
/// so the handler can short-circuit the render.
///
/// # Example
///
/// ```ignore
/// use std::time::Duration;
/// use tokio::sync::mpsc;
/// use webui::streaming::StreamingWriter;
///
/// let (tx, mut rx) = mpsc::channel(StreamingWriter::DEFAULT_CHANNEL_CAPACITY);
/// actix_web::rt::task::spawn_blocking(move || {
///     let mut writer = StreamingWriter::new(tx)
///         .with_flush_timeout(Duration::from_secs(30));
///     handler.render(&protocol, &state, &opts, &mut writer);
///     let _ = ResponseWriter::end(&mut writer);
/// });
/// // … wrap rx in a Stream and pass to HttpResponse::streaming …
/// ```
pub struct StreamingWriter {
    tx: Sender<Bytes>,
    buf: Vec<u8>,
    chunk_target: usize,
    /// Maximum time `flush_buf` may park on the channel. `None` =
    /// unbounded (backwards-compatible default).
    flush_timeout: Option<Duration>,
    /// Cached terminal error set after the first failed send/timeout,
    /// so subsequent `write()` calls short-circuit without paying for
    /// another atomic round-trip on the channel.
    terminated: Option<TerminationCause>,
    /// Optional shared chunk pool. When set, every flushed chunk is
    /// wrapped in a [`PooledChunk`] owner so its allocation returns
    /// to the pool when the consumer drops the `Bytes`. The next
    /// chunk buffer is acquired from the pool instead of being
    /// freshly allocated. See [`ChunkPool`] for the cost model.
    pool: Option<Arc<ChunkPool>>,
}

/// Reason a `StreamingWriter` is terminated. Stored unit-style; no
/// payload allocation per failed write.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum TerminationCause {
    Disconnected,
    Timeout,
}

impl From<TerminationCause> for HandlerError {
    fn from(cause: TerminationCause) -> Self {
        match cause {
            TerminationCause::Disconnected => HandlerError::ClientDisconnected,
            TerminationCause::Timeout => HandlerError::StreamTimeout,
        }
    }
}

impl StreamingWriter {
    /// Default chunk-coalescing target. ~4 KB is a balance: large enough
    /// to amortise per-call channel + actix overhead, small enough that
    /// head/body content arrives before the body_end script.
    ///
    /// Tunable via [`with_chunk_size`](Self::with_chunk_size).
    pub const CHUNK_TARGET: usize = 4 * 1024;

    /// Default bounded-channel capacity in chunks. With
    /// `CHUNK_TARGET = 4 KB`, this caps in-flight memory at ~16 KB per
    /// in-progress request.
    pub const DEFAULT_CHANNEL_CAPACITY: usize = 4;

    /// Minimum allowed chunk size. Below this the per-flush channel
    /// overhead dominates the payload cost.
    const MIN_CHUNK_TARGET: usize = 64;

    /// Slack added to the chunk buffer's capacity beyond `chunk_target`,
    /// to absorb a single oversized write without an immediate growth
    /// realloc. 1 KiB is comfortably above the largest single
    /// `ResponseWriter::write` call the WebUI handler emits in
    /// practice (signal values, attribute values, raw fragments are
    /// all small).
    const BUF_HEADROOM: usize = 1024;

    /// Wrap a tokio mpsc sender. Each render allocates its own chunk
    /// buffers via the system allocator. For pooled allocation across
    /// requests, use [`new_pooled`](Self::new_pooled).
    #[must_use]
    pub fn new(tx: Sender<Bytes>) -> Self {
        Self {
            tx,
            buf: Vec::with_capacity(Self::CHUNK_TARGET + Self::BUF_HEADROOM),
            chunk_target: Self::CHUNK_TARGET,
            flush_timeout: None,
            terminated: None,
            pool: None,
        }
    }

    /// Wrap a tokio mpsc sender, drawing chunk buffers from the
    /// shared `pool`. Recycled buffers eliminate per-flush allocation
    /// in steady-state high-RPS workloads. The pool is shared via
    /// `Arc` and is safe to use from any number of concurrent
    /// `StreamingWriter` instances; release happens when the consumer
    /// drops the `Bytes`, on whichever thread held the last reference.
    ///
    /// `chunk_target` defaults to [`CHUNK_TARGET`](Self::CHUNK_TARGET);
    /// override with [`with_chunk_size`](Self::with_chunk_size). When
    /// the pool's chunk size disagrees with the writer's target, the
    /// writer grows the acquired buffer on first use (one-time cost).
    #[must_use]
    pub fn new_pooled(tx: Sender<Bytes>, pool: Arc<ChunkPool>) -> Self {
        let buf = pool.acquire();
        Self {
            tx,
            buf,
            chunk_target: Self::CHUNK_TARGET,
            flush_timeout: None,
            terminated: None,
            pool: Some(pool),
        }
    }

    /// Override the chunk-coalescing target. Larger chunks reduce
    /// channel + syscall overhead at the cost of higher first-byte
    /// latency. Values below 64 bytes are silently raised to 64.
    ///
    /// Common sizes:
    /// - **1 KB**: minimise TTFB on small pages.
    /// - **4 KB** (default): balanced for ~24 KB SSR pages.
    /// - **16 KB**: match TLS record size for large SSR (>200 KB).
    #[must_use]
    pub fn with_chunk_size(mut self, bytes: usize) -> Self {
        let target = bytes.max(Self::MIN_CHUNK_TARGET);
        self.chunk_target = target;
        // Re-initialise the buffer at the new target. If pooled, the
        // current buffer goes back to the pool (it may be wrong-sized
        // for this writer, but other writers can still use it).
        let old = std::mem::replace(
            &mut self.buf,
            Vec::with_capacity(target + Self::BUF_HEADROOM),
        );
        if let Some(pool) = self.pool.as_ref() {
            pool.release(old);
        }
        self
    }

    /// Cap the maximum time a flush may park on the channel before
    /// returning [`HandlerError::StreamTimeout`]. `None` (default) means
    /// flushes block indefinitely on slow consumers.
    ///
    /// Production HTTP hosts should set this (e.g. 30 s) so a single
    /// slow-loris client cannot pin a render thread forever. The chosen
    /// timeout × concurrent-render-limit is the upper bound on resources
    /// an attacker can pin.
    ///
    /// Requires an active tokio runtime to be in TLS (i.e. the writer
    /// is being driven from a `spawn_blocking` task on a tokio runtime).
    /// Without one, the timeout is silently ignored and a plain
    /// `blocking_send` is performed.
    #[must_use]
    pub fn with_flush_timeout(mut self, timeout: Duration) -> Self {
        self.flush_timeout = Some(timeout);
        self
    }

    /// Send the current buffer as a chunk. Returns `Err` and marks the
    /// writer terminated on disconnect or timeout.
    fn flush_buf(&mut self) -> Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        if let Some(cause) = self.terminated {
            return Err(cause.into());
        }
        // Take the current buffer; immediately install the next one
        // (pool-acquired or freshly allocated) so subsequent writes
        // don't need to grow on the fly.
        let chunk = std::mem::take(&mut self.buf);
        self.buf = match self.pool.as_ref() {
            Some(pool) => {
                // `acquire` returns a buffer with at least `chunk_size`
                // capacity (clamped at construction); grow if our
                // chunk_target was overridden to be larger.
                let mut next = pool.acquire();
                let want = self.chunk_target + Self::BUF_HEADROOM;
                if next.capacity() < want {
                    next.reserve(want - next.capacity());
                }
                next
            }
            None => Vec::with_capacity(self.chunk_target + Self::BUF_HEADROOM),
        };

        // Build the payload. Pooled chunks wrap the Vec in a
        // PooledChunk owner so the buffer returns to the pool on
        // last-Bytes-drop. Unpooled chunks move the Vec into Bytes
        // directly (zero-copy via Bytes::from).
        let payload = match self.pool.as_ref() {
            Some(pool) => Bytes::from_owner(PooledChunk::new(chunk, Arc::clone(pool))),
            None => Bytes::from(chunk),
        };

        let outcome = send_with_optional_timeout(&self.tx, payload, self.flush_timeout);
        match outcome {
            SendOutcome::Ok => Ok(()),
            SendOutcome::Disconnected => {
                self.terminated = Some(TerminationCause::Disconnected);
                Err(HandlerError::ClientDisconnected)
            }
            SendOutcome::TimedOut => {
                self.terminated = Some(TerminationCause::Timeout);
                Err(HandlerError::StreamTimeout)
            }
        }
    }

    /// True after the writer has been terminated by a disconnect or
    /// flush timeout.
    #[must_use]
    pub fn is_terminated(&self) -> bool {
        self.terminated.is_some()
    }
}

impl Drop for StreamingWriter {
    fn drop(&mut self) {
        // Return the still-empty next-chunk buffer to the pool so it
        // doesn't fall on the floor at end-of-render. After the final
        // `flush_buf`, `self.buf` is the freshly-acquired or freshly-
        // allocated next buffer; if the render ended without filling
        // it, releasing it keeps the pool's working set warm.
        if let Some(pool) = self.pool.as_ref() {
            let buf = std::mem::take(&mut self.buf);
            // Only return non-trivial allocations; an empty Vec carries
            // no allocation (Vec::new) and would just churn the queue.
            if buf.capacity() > 0 {
                pool.release(buf);
            }
        }
    }
}

enum SendOutcome {
    Ok,
    Disconnected,
    TimedOut,
}

/// Send a chunk via blocking_send, optionally bounded by `timeout`.
///
/// When `timeout` is `Some` and a tokio runtime is in TLS, this uses
/// `Handle::block_on(timeout(send))`. The `block_on` is legal here only
/// when called from a `spawn_blocking` worker — not from inside async
/// code — so the writer's documented usage pattern is required.
///
/// When `timeout` is `None` we skip the runtime-handle TLS lookup
/// entirely (saves ~10 ns/flush; meaningful at 10k+ RPS).
///
/// **Slow-loris guard fail-safety.** If `timeout` is `Some` but no
/// tokio runtime is in TLS, we MUST NOT silently fall through to an
/// unbounded `blocking_send` — that would defeat the documented
/// slow-loris bound (`timeout × concurrent_renders`). Instead we
/// emit a `log::warn!` once per process so operators see the
/// misconfiguration, then enforce the deadline ourselves with a
/// runtime-free `try_send` + `std::thread::sleep` poll loop. The
/// poll interval is short relative to the typical timeout (30 s in
/// production), so the worst-case wakeup overshoot is bounded.
fn send_with_optional_timeout(
    tx: &Sender<Bytes>,
    payload: Bytes,
    timeout: Option<Duration>,
) -> SendOutcome {
    // Fast path: most production writers don't set a timeout.
    let Some(deadline) = timeout else {
        return match tx.blocking_send(payload) {
            Ok(()) => SendOutcome::Ok,
            Err(_) => SendOutcome::Disconnected,
        };
    };
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        let timed =
            handle.block_on(async { tokio::time::timeout(deadline, tx.send(payload)).await });
        return match timed {
            Ok(Ok(())) => SendOutcome::Ok,
            Ok(Err(_)) => SendOutcome::Disconnected,
            Err(_) => SendOutcome::TimedOut,
        };
    }
    // No runtime in TLS. Calling `tx.blocking_send` from inside a
    // tokio worker that's not `spawn_blocking` would panic ("Cannot
    // block the current thread from within a runtime") and with
    // `panic = "abort"` in the workspace release profile that aborts
    // the whole process. Calling it from a raw `std::thread::spawn`
    // would silently disable the slow-loris bound. Neither is
    // acceptable, so we enforce the deadline ourselves with a
    // try_send poll loop.
    no_runtime_timeout_warn_once();
    runtime_free_send(tx, payload, deadline)
}

fn no_runtime_timeout_warn_once() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static WARNED: AtomicBool = AtomicBool::new(false);
    if !WARNED.swap(true, Ordering::Relaxed) {
        log::warn!(
            "StreamingWriter::with_flush_timeout was set, but no tokio runtime is in TLS. \
             Falling back to a runtime-free poll loop (slow-loris bound is preserved but \
             with a small wakeup overshoot). Wire the writer from `spawn_blocking` to use \
             the precise tokio path."
        );
    }
}

/// Runtime-free deadline-bounded send. Polls `try_send` with a
/// short `thread::sleep` between attempts. The poll interval is
/// 1 ms, so wakeup overshoot vs the configured deadline is bounded
/// by 1 ms — negligible compared to the typical 30 s production
/// timeout. Backs off to a longer interval after the first second
/// to keep idle CPU low for large timeouts.
fn runtime_free_send(tx: &Sender<Bytes>, payload: Bytes, deadline: Duration) -> SendOutcome {
    use tokio::sync::mpsc::error::TrySendError;
    let start = Instant::now();
    let mut payload = payload;
    let mut interval = Duration::from_millis(1);
    let backoff_after = Duration::from_secs(1);
    loop {
        match tx.try_send(payload) {
            Ok(()) => return SendOutcome::Ok,
            Err(TrySendError::Closed(_)) => return SendOutcome::Disconnected,
            Err(TrySendError::Full(returned)) => {
                if start.elapsed() >= deadline {
                    return SendOutcome::TimedOut;
                }
                std::thread::sleep(interval);
                if start.elapsed() >= backoff_after && interval < Duration::from_millis(50) {
                    interval = Duration::from_millis(50);
                }
                payload = returned;
            }
        }
    }
}

impl ResponseWriter for StreamingWriter {
    fn write(&mut self, content: &str) -> Result<()> {
        if let Some(cause) = self.terminated {
            return Err(cause.into());
        }
        self.buf.extend_from_slice(content.as_bytes());
        if self.buf.len() >= self.chunk_target {
            self.flush_buf()?;
        }
        Ok(())
    }

    fn end(&mut self) -> Result<()> {
        // Surface the final-flush error so the caller can distinguish
        // "fully delivered" from "client gave up at the very last
        // chunk." If `terminated` is already set, `write()` already
        // surfaced the error earlier — return Ok here so the caller
        // doesn't see the same disconnect twice.
        //
        // This is the contract that motivated introducing
        // `HandlerError::ClientDisconnected` / `StreamTimeout` in the
        // first place: callers want a programmatic signal so they can
        // decrement `render_errors_total` correctly and avoid logging
        // truncated responses as 200-OK successes.
        if self.terminated.is_some() {
            return Ok(());
        }
        self.flush_buf()
    }
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── StreamingWriter tests ───────────────────────────────────────

    fn drain(mut rx: tokio::sync::mpsc::Receiver<Bytes>) -> String {
        let mut buf = Vec::new();
        while let Ok(chunk) = rx.try_recv() {
            buf.extend_from_slice(&chunk);
        }
        String::from_utf8(buf).expect("valid utf-8")
    }

    #[test]
    fn streaming_writer_coalesces_small_writes() {
        let (tx, rx) = tokio::sync::mpsc::channel::<Bytes>(8);
        let mut w = StreamingWriter::new(tx);
        for _ in 0..10 {
            ResponseWriter::write(&mut w, "abc").unwrap();
        }
        ResponseWriter::end(&mut w).unwrap();
        drop(w);
        assert_eq!(drain(rx), "abc".repeat(10));
    }

    #[test]
    fn streaming_writer_flushes_at_chunk_boundary() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Bytes>(8);
        let mut w = StreamingWriter::new(tx);
        let big = "x".repeat(StreamingWriter::CHUNK_TARGET);
        ResponseWriter::write(&mut w, &big).unwrap();
        let first = rx.try_recv().expect("first chunk should be available");
        assert_eq!(first.len(), StreamingWriter::CHUNK_TARGET);
        ResponseWriter::end(&mut w).unwrap();
    }

    #[test]
    fn streaming_writer_returns_typed_error_after_disconnect() {
        let (tx, rx) = tokio::sync::mpsc::channel::<Bytes>(1);
        let mut w = StreamingWriter::new(tx);
        drop(rx);
        ResponseWriter::write(&mut w, "hi").unwrap();
        let big = "x".repeat(StreamingWriter::CHUNK_TARGET);
        let result = ResponseWriter::write(&mut w, &big);
        assert!(matches!(result, Err(HandlerError::ClientDisconnected)));
        assert!(w.is_terminated());
        let result2 = ResponseWriter::write(&mut w, "more");
        assert!(matches!(result2, Err(HandlerError::ClientDisconnected)));
    }

    #[test]
    fn streaming_writer_end_after_disconnect_succeeds() {
        let (tx, rx) = tokio::sync::mpsc::channel::<Bytes>(1);
        let mut w = StreamingWriter::new(tx);
        drop(rx);
        let big = "x".repeat(StreamingWriter::CHUNK_TARGET);
        let _ = ResponseWriter::write(&mut w, &big);
        assert!(w.is_terminated());
        // Already-terminated end() returns Ok — the error was already
        // surfaced via write() and the caller acted on it.
        ResponseWriter::end(&mut w).unwrap();
    }

    /// Regression for the bug Akrosh caught: when the writer hasn't
    /// yet flushed (sub-`chunk_target` content) and the receiver has
    /// disconnected, `end()` MUST surface the typed error rather than
    /// silently returning `Ok(())` and lying to the caller about a
    /// successful response.
    #[test]
    fn streaming_writer_end_surfaces_first_flush_error() {
        let (tx, rx) = tokio::sync::mpsc::channel::<Bytes>(1);
        let mut w = StreamingWriter::new(tx);
        drop(rx);
        // Below `chunk_target` — no automatic flush from write(),
        // so `terminated` is None at the time end() runs.
        ResponseWriter::write(&mut w, "small").unwrap();
        assert!(!w.is_terminated(), "no automatic flush yet");

        let result = ResponseWriter::end(&mut w);
        assert!(
            matches!(result, Err(HandlerError::ClientDisconnected)),
            "end() must surface ClientDisconnected from final flush, got {result:?}"
        );
        assert!(w.is_terminated(), "writer must be marked terminated");
    }

    #[test]
    fn streaming_writer_custom_chunk_size() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Bytes>(8);
        let mut w = StreamingWriter::new(tx).with_chunk_size(128);
        ResponseWriter::write(&mut w, &"x".repeat(127)).unwrap();
        assert!(rx.try_recv().is_err(), "below threshold, no flush yet");
        ResponseWriter::write(&mut w, "x").unwrap();
        let first = rx.try_recv().expect("chunk should flush at 128 bytes");
        assert_eq!(first.len(), 128);
    }

    #[test]
    fn streaming_writer_min_chunk_size_clamp() {
        let (tx, _rx) = tokio::sync::mpsc::channel::<Bytes>(8);
        let w = StreamingWriter::new(tx).with_chunk_size(1);
        assert_eq!(w.chunk_target, StreamingWriter::MIN_CHUNK_TARGET);
    }

    /// Positive test for the slow-loris guard. Without a tokio runtime
    /// in TLS, `with_flush_timeout` is forced down the runtime-free
    /// poll-loop path. Fill a 1-slot channel without consuming it,
    /// then verify the writer surfaces `Err(StreamTimeout)` after the
    /// configured deadline (and does NOT silently fall through to an
    /// untimed `blocking_send` as the previous implementation did).
    ///
    /// Akrosh's review caught this gap: the slow-loris bound was
    /// previously the framework's only DoS guard but had no positive
    /// test, and the fallback path was a `debug_assert!(false)` that
    /// compiled to a no-op in release.
    #[test]
    fn streaming_writer_flush_timeout_fires_without_runtime() {
        let (tx, _rx) = tokio::sync::mpsc::channel::<Bytes>(1);
        let mut w = StreamingWriter::new(tx)
            .with_chunk_size(64)
            .with_flush_timeout(Duration::from_millis(150));

        // Fill the 1-slot channel.
        ResponseWriter::write(&mut w, &"x".repeat(64)).unwrap();
        // Next flush has nowhere to go → must time out within
        // ~deadline + 1 ms (poll interval). Allow a generous CI cushion.
        let start = Instant::now();
        let result = ResponseWriter::write(&mut w, &"y".repeat(64));
        let elapsed = start.elapsed();

        assert!(
            matches!(result, Err(HandlerError::StreamTimeout)),
            "expected Err(StreamTimeout), got {result:?}"
        );
        assert!(
            elapsed >= Duration::from_millis(150),
            "must wait at least the deadline; elapsed={elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(1500),
            "must not block much past the deadline; elapsed={elapsed:?}"
        );
        assert!(w.is_terminated(), "writer must be marked terminated");

        // Subsequent writes short-circuit (no second timeout wait).
        let start = Instant::now();
        let result2 = ResponseWriter::write(&mut w, "more");
        assert!(matches!(result2, Err(HandlerError::StreamTimeout)));
        assert!(
            start.elapsed() < Duration::from_millis(50),
            "subsequent writes must short-circuit; elapsed={:?}",
            start.elapsed()
        );
    }

    // ── ChunkPool tests ─────────────────────────────────────────────

    /// Acquire/release round-trip: a buffer pushed into the pool comes
    /// back out empty with at least the requested capacity.
    #[test]
    fn pool_round_trip() {
        let pool = ChunkPool::new(4, 1024);
        let buf = pool.acquire();
        assert!(buf.capacity() >= 1024);
        assert_eq!(buf.len(), 0);
        assert_eq!(pool.idle_count(), 0);

        pool.release(buf);
        assert_eq!(pool.idle_count(), 1);

        // Second acquire returns the released buffer (capacity preserved).
        let buf2 = pool.acquire();
        assert!(buf2.capacity() >= 1024);
        assert_eq!(pool.idle_count(), 0);
    }

    /// A non-empty buffer released to the pool must come back empty
    /// (defensive `clear()` in `acquire`).
    #[test]
    fn pool_clears_dirty_buffer_on_acquire() {
        let pool = ChunkPool::new(2, 16);
        let mut dirty = Vec::with_capacity(64);
        dirty.extend_from_slice(b"leftover content");
        pool.release(dirty);

        let acquired = pool.acquire();
        assert_eq!(acquired.len(), 0, "acquired buffer must be empty");
    }

    /// Pool capacity is enforced — overflow buffers are dropped, not
    /// queued.
    #[test]
    fn pool_full_drops_excess() {
        let pool = ChunkPool::new(2, 8);
        pool.release(Vec::with_capacity(8));
        pool.release(Vec::with_capacity(8));
        assert_eq!(pool.idle_count(), 2);
        // Third release would exceed capacity; queue rejects it silently.
        pool.release(Vec::with_capacity(8));
        assert_eq!(pool.idle_count(), 2, "pool must not grow beyond capacity");
    }

    /// `PooledChunk` must return its buffer to the pool when dropped.
    /// This is the lifecycle the production path depends on:
    /// `Bytes::from_owner(PooledChunk)` keeps the chunk alive while
    /// the actix worker writes it to the wire; on the worker's drop,
    /// the buffer recycles.
    #[test]
    fn pooled_chunk_drop_returns_to_pool() {
        let pool = Arc::new(ChunkPool::new(4, 256));
        assert_eq!(pool.idle_count(), 0);

        let buf = pool.acquire();
        assert_eq!(pool.idle_count(), 0);

        let payload = Bytes::from_owner(PooledChunk::new(buf, Arc::clone(&pool)));
        // Bytes is alive → buffer is "in flight".
        assert_eq!(pool.idle_count(), 0);

        drop(payload);
        // Buffer returned.
        assert_eq!(pool.idle_count(), 1);
    }

    /// Cloning a `Bytes` shares the chunk; only when the LAST clone
    /// drops does the buffer return to the pool. This models the
    /// actix → tcp pipeline where multiple internal layers may hold
    /// references.
    #[test]
    fn pooled_chunk_returns_after_last_clone_drop() {
        let pool = Arc::new(ChunkPool::new(4, 256));
        let buf = pool.acquire();
        let original = Bytes::from_owner(PooledChunk::new(buf, Arc::clone(&pool)));
        let clone1 = original.clone();
        let clone2 = original.clone();
        assert_eq!(pool.idle_count(), 0);

        drop(original);
        assert_eq!(pool.idle_count(), 0, "still 2 refs alive");
        drop(clone1);
        assert_eq!(pool.idle_count(), 0, "still 1 ref alive");
        drop(clone2);
        assert_eq!(pool.idle_count(), 1, "last ref dropped, buffer returned");
    }

    /// `StreamingWriter::new_pooled` recycles its chunk buffers across
    /// successive renders that share the same pool. After the first
    /// render fills the pool, subsequent renders should not allocate
    /// fresh chunk buffers.
    #[test]
    fn streaming_writer_pooled_recycles_buffers() {
        let pool = Arc::new(ChunkPool::new(8, StreamingWriter::CHUNK_TARGET));

        // First render: pool starts empty, every flush allocates.
        {
            let (tx, rx) = tokio::sync::mpsc::channel::<Bytes>(8);
            let mut w = StreamingWriter::new_pooled(tx, Arc::clone(&pool));
            for _ in 0..3 {
                ResponseWriter::write(&mut w, &"x".repeat(StreamingWriter::CHUNK_TARGET)).unwrap();
            }
            ResponseWriter::end(&mut w).unwrap();
            drop(w);
            // Drain the channel — drops the Bytes → returns chunks to pool.
            let _ = drain(rx);
        }
        let after_first = pool.idle_count();
        assert!(
            after_first >= 3,
            "after first render, pool should have ≥3 buffers; got {after_first}"
        );

        // Second render: should reuse pooled buffers.
        let before_second = pool.idle_count();
        {
            let (tx, rx) = tokio::sync::mpsc::channel::<Bytes>(8);
            let mut w = StreamingWriter::new_pooled(tx, Arc::clone(&pool));
            for _ in 0..3 {
                ResponseWriter::write(&mut w, &"x".repeat(StreamingWriter::CHUNK_TARGET)).unwrap();
            }
            ResponseWriter::end(&mut w).unwrap();
            drop(w);
            let _ = drain(rx);
        }
        let after_second = pool.idle_count();
        // Idle count should be steady — every buffer acquired during the
        // second render came back at the end.
        assert!(
            after_second >= before_second.saturating_sub(1),
            "pool should not shrink across renders: before={before_second} after={after_second}"
        );
    }

    /// Cross-thread drop safety: a `PooledChunk` built on thread A
    /// can be dropped on thread B, and the buffer returns to the
    /// shared pool correctly. This is the actix scenario (producer
    /// is `spawn_blocking`, consumer drops on the I/O worker).
    #[test]
    fn pooled_chunk_cross_thread_drop() {
        let pool = Arc::new(ChunkPool::new(4, 128));
        let buf = pool.acquire();
        let payload = Bytes::from_owner(PooledChunk::new(buf, Arc::clone(&pool)));

        let pool_for_thread = Arc::clone(&pool);
        let h = std::thread::spawn(move || {
            // Drop on the spawned thread.
            drop(payload);
            // Verify drop ran by checking idle count from this thread.
            assert_eq!(pool_for_thread.idle_count(), 1);
        });
        h.join().unwrap();
        // Main thread sees the recycled buffer too.
        assert_eq!(pool.idle_count(), 1);
    }
}
