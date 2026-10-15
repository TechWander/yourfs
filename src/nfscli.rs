//! Synchronous façade over the libnfs async API, with request pipelining.
//!
//! ## Why a service thread per connection
//! libnfs's `struct nfs_context` is one TCP connection with private RPC
//! state and is **not** thread-safe: every call on a context must come from
//! one thread at a time. The previous design held a per-slot mutex across
//! the whole issue+wait span, which is safe but strictly serial: on a link
//! with even 10 ms latency a single file was read one 128 KiB FUSE request
//! at a time (measured 13x slower than a kernel NFS mount for the same
//! stream, because the kernel client pipelines many READs per round trip).
//!
//! NFS-over-TCP allows exactly that pipelining: several RPCs may be queued
//! on one connection and libnfs dispatches the replies as they arrive. To
//! exploit it every context gets a dedicated service thread (actor model):
//! workers hand their operation to the actor through a channel and park on
//! a private reply channel; the actor issues *everything that is queued*
//! back-to-back without waiting, then pumps `nfs_service()` until the
//! connection goes quiet. N concurrent reads complete in about one round
//! trip instead of N, both across threads and inside one file's stream.
//!
//! ## File handles stay pinned to their context
//! `nfs_open` returns an `nfsfh` carrying that connection's RPC state, so
//! `pread`/`close` are routed back to the owning slot through `open_files`
//! (lookup under the map lock only, never while touching a context).
//!
//! ## Reply plumbing
//! Each in-flight operation owns a heap `Pending` (the C-side `SyncCbData`
//! plus a reply channel and a slot in-flight counter). The libnfs callback
//! runs on the actor thread, copies results into buffers the caller still
//! owns, then wakes the caller through the channel. A `Pending` is consumed
//! by exactly one of two paths: the callback, or the issue-failed path in
//! the actor (a rejected async call never queues, so it never fires).
//! libnfs also completes every pending call with an error when the socket
//! dies, so a worker never waits on a reply that cannot come.

use std::collections::HashMap;
use std::ffi::CStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;

use libc::{c_int, c_void};

use crate::libnfs::*;
use crate::paths::to_cstring;

/// Default number of NFS connections when `-V/--nfs_connections` is not given.
/// Acts as the upper bound: if the server refuses the extra mounts we simply
/// run with fewer (at least one).
pub const DEFAULT_POOL_SIZE: usize = 4;

/// Raw pointer moved deliberately across a thread boundary. Sound only for
/// the two uses here: an `nfs_context` handed to its own actor thread, and an
/// `nfsfh` handed back to the actor that owns its context — in both cases the
/// pointer is only ever dereferenced on the single thread that receives it.
struct SendPtr<T> {
    p: *mut T,
}
unsafe impl<T> Send for SendPtr<T> {}

impl<T> SendPtr<T> {
    /// Consume the wrapper and yield the pointer. Callers inside a closure
    /// must use this instead of the field path `sp.p`: edition-2021 precise
    /// captures would otherwise move the bare `*mut T` (not `Send`) into the
    /// closure and reject the spawn.
    fn raw(self) -> *mut T {
        self.p
    }
}

/// Immutable, shared RPC credential configuration (identical for every slot).
struct Cfg {
    custom_uid: i32,
    custom_gid: i32,
    allow_other_own_ids: bool,
    mount_user_uid: u32,
    mount_user_gid: u32,
}

/// Completion payload a libnfs callback delivers to its waiting caller.
pub struct Reply {
    pub status: i32,
    /// only `open` uses this: the returned `nfsfh` pointer
    pub ptr: *mut c_void,
}
unsafe impl Send for Reply {}

impl Reply {
    fn err(status: i32) -> Self {
        Reply {
            status,
            ptr: std::ptr::null_mut(),
        }
    }
}

/// Heap state of one in-flight operation, handed to libnfs as `private_data`.
/// `cb` is only ever touched by the actor thread (through the C callback that
/// receives this pointer); `reply` crosses to the worker by design.
struct Pending {
    cb: SyncCbData,
    reply: Sender<Reply>,
    inflight: Arc<AtomicUsize>,
}
unsafe impl Send for Pending {}

/// One queued operation. `Quit` stops the actor thread.
enum Task {
    Quit,
    Op {
        /// (uid, gid) to install on the context before issuing; None keeps
        /// the current ids (statvfs/close do not set credentials, as in C).
        creds: Option<(u32, u32)>,
        /// issues the async libnfs call; returns <0 when the request was
        /// not queued (then the actor completes the Pending itself)
        issue: Box<dyn FnOnce(*mut nfs_context, *mut Pending) -> i32 + Send>,
        pending: Box<Pending>,
    },
}

/// Run the matching libnfs callback (it writes `p.cb` and copies results
/// into buffers the caller still owns), then complete the pending operation.
/// Always runs on the actor thread, from inside `nfs_service`.
fn complete(
    mut p: Pending,
    base: extern "C" fn(c_int, *mut nfs_context, *mut c_void, *mut c_void),
    status: c_int,
    nfs: *mut nfs_context,
    data: *mut c_void,
) {
    base(status, nfs, data, &mut p.cb as *mut SyncCbData as *mut c_void);
    // read the possibly-clamped status back (read_cb corrects an over-long
    // reply to what actually fits the caller buffer)
    let status = p.cb.status;
    let ptr = p.cb.return_data;
    p.inflight.fetch_sub(1, Ordering::SeqCst);
    let reply = p.reply.clone();
    drop(p); // frees the Pending while the message already lives in the queue
    let _ = reply.send(Reply { status, ptr });
}

extern "C" fn done_generic(status: c_int, nfs: *mut nfs_context, data: *mut c_void, pd: *mut c_void) {
    let p = unsafe { *Box::from_raw(pd as *mut Pending) };
    complete(p, generic_cb, status, nfs, data)
}
extern "C" fn done_struct(status: c_int, nfs: *mut nfs_context, data: *mut c_void, pd: *mut c_void) {
    let p = unsafe { *Box::from_raw(pd as *mut Pending) };
    complete(p, copy_struct_cb, status, nfs, data)
}
extern "C" fn done_readlink(status: c_int, nfs: *mut nfs_context, data: *mut c_void, pd: *mut c_void) {
    let p = unsafe { *Box::from_raw(pd as *mut Pending) };
    complete(p, readlink_cb, status, nfs, data)
}
extern "C" fn done_open(status: c_int, nfs: *mut nfs_context, data: *mut c_void, pd: *mut c_void) {
    let p = unsafe { *Box::from_raw(pd as *mut Pending) };
    complete(p, open_cb, status, nfs, data)
}
extern "C" fn done_read(status: c_int, nfs: *mut nfs_context, data: *mut c_void, pd: *mut c_void) {
    let p = unsafe { *Box::from_raw(pd as *mut Pending) };
    complete(p, read_cb, status, nfs, data)
}

/// one slot = one nfs_context + its dedicated actor thread
struct Slot {
    /// only the actor thread dereferences this (or shutdown after joining it)
    ctx: *mut nfs_context,
    tx: Sender<Task>,
    inflight: Arc<AtomicUsize>,
    cfg: Arc<Cfg>,
    /// The receiver and the actor handle live here, guarded by one mutex so
    /// the actor is spawned exactly once. Spawning is *lazy* on purpose:
    /// libfuse daemonizes by forking inside `fuse_main`, and fork only keeps
    /// the calling thread — an actor spawned before that point would vanish
    /// in the daemon and every operation would block on a reply nobody
    /// could ever send. The first real operation runs well after the fork.
    state: Mutex<SlotState>,
}

struct SlotState {
    rx: Option<Receiver<Task>>,
    thread: Option<JoinHandle<()>>,
}

impl Slot {
    /// Spawn this slot's actor thread if it is not running yet.
    fn ensure_started(&self) {
        let mut st = self.state.lock().unwrap();
        let Some(rx) = st.rx.take() else { return };
        let send = SendPtr { p: self.ctx };
        let cfg = Arc::clone(&self.cfg);
        let inflight = Arc::clone(&self.inflight);
        st.thread = Some(
            std::thread::Builder::new()
                .name("nfs-slot".to_string())
                .spawn(move || runner(send.raw(), &cfg, rx, &inflight))
                .expect("spawn nfs actor"),
        );
    }
}
unsafe impl Send for Slot {}
unsafe impl Sync for Slot {}

pub struct NfsPool {
    slots: Vec<Slot>,
    next: AtomicUsize,
    /// `nfsfh` address -> index of the slot (context) that opened it
    open_files: Mutex<HashMap<usize, usize>>,
}

static NFS: OnceLock<NfsPool> = OnceLock::new();

fn pool() -> &'static NfsPool {
    NFS.get().expect("nfs not initialized")
}

/// Mount a fresh context against `server`/`export`. Returns the raw pointer on
/// success, tearing the context down on failure.
fn mount_extra(server: &CStr, export: &CStr) -> Option<*mut nfs_context> {
    let ctx = unsafe { nfs_init_context() };
    if ctx.is_null() {
        return None;
    }
    let ret = unsafe { nfs_mount(ctx, server.as_ptr(), export.as_ptr()) };
    if ret != 0 {
        unsafe { nfs_destroy_context(ctx) };
        return None;
    }
    Some(ctx)
}

/// Initialize the connection pool. `first_ctx` is an already-mounted context
/// (created by the caller so URL parse / mount errors surface at startup with
/// the right exit code); we open up to `pool_size - 1` more alongside it.
/// `pool_size == 1` keeps a single connection (still pipelined).
// one caller-supplied context plus the seven mount/credential settings that
// bin/yourfs.rs already passes positionally; grouping them would only add
// indirection without removing a single parameter from the actual call.
#[allow(clippy::too_many_arguments)]
pub fn init(
    first_ctx: *mut nfs_context,
    server: &CStr,
    export: &CStr,
    custom_uid: i32,
    custom_gid: i32,
    allow_other_own_ids: bool,
    mount_user_uid: u32,
    mount_user_gid: u32,
    pool_size: usize,
) {
    let mut ctxs = vec![first_ctx];
    for _ in 1..pool_size.max(1) {
        match mount_extra(server, export) {
            Some(ctx) => ctxs.push(ctx),
            None => break, // server limit / transient error: run with fewer
        }
    }
    let cfg = Arc::new(Cfg {
        custom_uid,
        custom_gid,
        allow_other_own_ids,
        mount_user_uid,
        mount_user_gid,
    });
    let slots = ctxs
        .into_iter()
        .map(|ctx| {
            let (tx, rx) = channel::<Task>();
            let inflight = Arc::new(AtomicUsize::new(0));
            Slot {
                ctx,
                tx,
                inflight,
                cfg: Arc::clone(&cfg),
                state: Mutex::new(SlotState {
                    rx: Some(rx),
                    thread: None,
                }),
            }
        })
        .collect();
    let _ = NFS.set(NfsPool {
        slots,
        next: AtomicUsize::new(0),
        open_files: Mutex::new(HashMap::new()),
    });
}

/// Tear down every pooled context, freeing the connections opened by `init`
/// (the first caller-supplied one plus each extra mount). Call exactly once
/// after the FUSE main loop returns, when no callback can touch the pool any
/// more. Each actor is asked to quit and joined before its context is
/// destroyed, so nothing can be mid-operation on it. This also means slot 0
/// (the caller's original context) is owned by the pool from `init` on and
/// must NOT be destroyed separately.
pub fn shutdown() {
    let Some(p) = NFS.get() else { return };
    for slot in &p.slots {
        let _ = slot.tx.send(Task::Quit);
        if let Some(handle) = slot.state.lock().unwrap().thread.take() {
            let _ = handle.join();
        }
        unsafe { nfs_destroy_context(slot.ctx) };
    }
    if let Ok(mut m) = p.open_files.lock() {
        m.clear();
    }
}

/// The actor loop for one context. All libnfs calls for this connection
/// happen here and nowhere else.
fn runner(ctx: *mut nfs_context, cfg: &Cfg, rx: Receiver<Task>, inflight: &AtomicUsize) {
    loop {
        // block until at least one task arrives...
        let task = match rx.recv() {
            Ok(task) => task,
            Err(_) => break, // all senders dropped (process teardown)
        };
        if !handle_task(ctx, cfg, task, inflight) {
            break;
        }
        // ...then take everything else already queued: issuing those
        // back-to-back is what puts multiple RPCs on the wire at once
        while let Ok(t) = rx.try_recv() {
            if !handle_task(ctx, cfg, t, inflight) {
                return;
            }
        }
        // pump replies until the connection goes quiet; work that arrives
        // meanwhile is issued immediately, keeping the queue full even while
        // the server is streaming
        while inflight.load(Ordering::SeqCst) > 0 {
            pump(ctx);
            while let Ok(t) = rx.try_recv() {
                if !handle_task(ctx, cfg, t, inflight) {
                    return;
                }
            }
        }
    }
}

/// Issue one task; returns false when it was the shutdown request.
fn handle_task(ctx: *mut nfs_context, cfg: &Cfg, task: Task, inflight: &AtomicUsize) -> bool {
    let Task::Op { creds, issue, pending } = task else {
        return false;
    };
    if let Some((uid, gid)) = creds {
        update_rpc_credentials(ctx, cfg, uid, gid);
    }
    let p = Box::into_raw(pending);
    // Count the request as in flight *before* issuing it: if libnfs ever
    // completes a call synchronously inside the *_async issue (an error
    // path), the callback's fetch_sub would otherwise run before our
    // fetch_add and underflow the counter, pinning the pump loop forever.
    inflight.fetch_add(1, Ordering::SeqCst);
    let ret = issue(ctx, p);
    if ret < 0 {
        inflight.fetch_sub(1, Ordering::SeqCst);
        // not queued: no callback will ever run, complete it here
        let p = unsafe { *Box::from_raw(p) };
        let reply = p.reply.clone();
        drop(p);
        let _ = reply.send(Reply::err(ret));
    }
    true
}

/// `update_rpc_credentials()` — runs on the actor thread before each issue.
fn update_rpc_credentials(ctx: *mut nfs_context, cfg: &Cfg, caller_uid: u32, caller_gid: u32) {
    // Both "use the caller's ids" conditions collapse to one boolean: the
    // plain no-custom-uid case, or allow_other with a caller that is not the
    // mount user. Same truth table as the C `update_rpc_credentials()`.
    let use_caller_uid = (cfg.custom_uid == -1 && !cfg.allow_other_own_ids)
        || (cfg.allow_other_own_ids && (cfg.custom_uid == -1 || caller_uid != cfg.mount_user_uid));
    let use_caller_gid = (cfg.custom_gid == -1 && !cfg.allow_other_own_ids)
        || (cfg.allow_other_own_ids && (cfg.custom_gid == -1 || caller_gid != cfg.mount_user_gid));
    unsafe {
        if use_caller_uid {
            nfs_set_uid(ctx, caller_uid as i32);
        } else {
            nfs_set_uid(ctx, cfg.custom_uid);
        }

        if use_caller_gid {
            nfs_set_gid(ctx, caller_gid as i32);
        } else {
            nfs_set_gid(ctx, cfg.custom_gid);
        }
    }
}

/// One round of wait-and-service on the connection. A socket error makes
/// libnfs complete every pending call with an error status (those callbacks
/// fire from inside `nfs_service`), so we never abort the pump early — that
/// would strand waiters.
fn pump(ctx: *mut nfs_context) {
    unsafe {
        let mut pfd: libc::pollfd = std::mem::zeroed();
        pfd.fd = nfs_get_fd(ctx);
        pfd.events = nfs_which_events(ctx) as i16;
        let ret = libc::poll(&mut pfd, 1, 100);
        if ret < 0 {
            // A signal (EINTR) while waiting is normal in a multithreaded
            // FUSE process; handing libnfs revents=-1 for it would look
            // like a socket error and fail the operation spuriously.
            if *libc::__errno_location() == libc::EINTR {
                return;
            }
            nfs_service(ctx, -1);
            return;
        }
        nfs_service(ctx, pfd.revents as i32);
    }
}

/// Pick a slot, rotating so concurrent workers spread over the connections.
fn pick(p: &NfsPool) -> usize {
    p.next.fetch_add(1, Ordering::Relaxed) % p.slots.len()
}

/// Hand one operation to a slot actor and wait for its private reply.
fn run_op(
    slot: &Slot,
    creds: Option<(u32, u32)>,
    cb: SyncCbData,
    issue: impl FnOnce(*mut nfs_context, *mut Pending) -> i32 + Send + 'static,
) -> Reply {
    let (tx, rx) = channel();
    let pending = Box::new(Pending {
        cb,
        reply: tx,
        inflight: Arc::clone(&slot.inflight),
    });
    let task = Task::Op {
        creds,
        issue: Box::new(issue),
        pending,
    };
    // first operation of the daemon (post-fork) starts the actor
    slot.ensure_started();
    if slot.tx.send(task).is_err() {
        return Reply::err(-libc::EIO); // actor gone
    }
    rx.recv().unwrap_or(Reply::err(-libc::EIO))
}

pub fn lstat64(path: &str, caller: (u32, u32)) -> Result<nfs_stat_64, i32> {
    let cpath = to_cstring(path);
    let st = Box::into_raw(Box::new(nfs_stat_64::default()));
    let cb = SyncCbData {
        return_data: st as *mut c_void,
        max_size: std::mem::size_of::<nfs_stat_64>(),
        ..Default::default()
    };
    let p = pool();
    let reply = run_op(&p.slots[pick(p)], Some(caller), cb, move |nfs, pcb| unsafe {
        nfs_lstat64_async(nfs, cpath.as_ptr(), done_struct, pcb as *mut c_void)
    });
    let st = unsafe { *Box::from_raw(st) };
    if reply.status < 0 {
        return Err(reply.status);
    }
    Ok(st)
}

pub fn readlink(path: &str, size: usize, caller: (u32, u32)) -> Result<String, i32> {
    let cpath = to_cstring(path);
    let mut buf = Box::new(vec![0u8; size + 1]);
    let cb = SyncCbData {
        return_data: buf.as_mut_ptr() as *mut c_void,
        max_size: size,
        ..Default::default()
    };
    let p = pool();
    let reply = run_op(&p.slots[pick(p)], Some(caller), cb, move |nfs, pcb| unsafe {
        nfs_readlink_async(nfs, cpath.as_ptr(), done_readlink, pcb as *mut c_void)
    });
    if reply.status < 0 {
        return Err(reply.status);
    }
    let n = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    Ok(String::from_utf8_lossy(&buf[..n]).into_owned())
}

/// Open on a chosen slot and remember which slot owns the handle, so later
/// reads/closes route back to the same connection.
///
/// Returns the opaque nfsfh pointer to store in `fuse_file_info.fh`.
pub fn open(path: &str, flags: i32, caller: (u32, u32)) -> Result<*mut nfsfh, i32> {
    let cpath = to_cstring(path);
    let p = pool();
    let i = pick(p);
    let reply = run_op(
        &p.slots[i],
        Some(caller),
        SyncCbData::default(),
        move |nfs, pcb| unsafe {
            nfs_open_async(nfs, cpath.as_ptr(), flags, done_open, pcb as *mut c_void)
        },
    );
    if reply.status < 0 {
        return Err(reply.status);
    }
    let fh = reply.ptr as *mut nfsfh;
    if fh.is_null() {
        // Success status but no handle: treat as an error, otherwise the null
        // lands in `open_files` under key 0 and a later pread would call
        // nfs_pread_async(NULL) and crash the mount process.
        return Err(-libc::EIO);
    }
    p.open_files.lock().unwrap().insert(fh as usize, i);
    Ok(fh)
}

/// Close using the context that opened the handle. The map removal happens
/// before the task is handed to the actor, never while the context is in use.
///
/// # Safety
/// `nfsfh` must be a handle returned by [`open`] that has not been closed
/// yet (the FUSE release-per-fh contract guarantees both).
pub unsafe fn close(nfsfh: *mut nfsfh) {
    let p = pool();
    let slot_idx = p.open_files.lock().unwrap().remove(&(nfsfh as usize));
    let Some(i) = slot_idx else { return };
    let fh = SendPtr { p: nfsfh };
    let _ = run_op(&p.slots[i], None, SyncCbData::default(), move |nfs, pcb| unsafe {
        nfs_close_async(nfs, fh.raw(), done_generic, pcb as *mut c_void)
    });
}

/// Returns the number of bytes read into `buf`, or a negative errno.
/// Reads must run on the context that owns `nfsfh`, so we look that slot up
/// first (briefly holding only the map lock); the actor then issues the RPC
/// while the caller parks on its reply channel.
///
/// # Safety
/// `nfsfh` must be a handle returned by [`open`] that is still open; unknown
/// handles fail with EIO via the `open_files` lookup before any dereference.
/// `buf` must stay alive for the whole call: the libnfs read callback (on
/// the actor thread) writes into it before the reply wakes this function.
pub unsafe fn pread(
    nfsfh: *mut nfsfh,
    offset: u64,
    buf: &mut [u8],
    caller: (u32, u32),
) -> Result<usize, i32> {
    let p = pool();
    let slot_idx = {
        let m = p.open_files.lock().unwrap();
        m.get(&(nfsfh as usize)).copied()
    };
    let Some(i) = slot_idx else {
        return Err(-libc::EIO);
    };
    let ptr = buf.as_mut_ptr();
    let len = buf.len();
    let fh = SendPtr { p: nfsfh };
    let cb = SyncCbData {
        return_data: ptr as *mut c_void,
        max_size: len,
        ..Default::default()
    };
    let reply = run_op(&p.slots[i], Some(caller), cb, move |nfs, pcb| unsafe {
        nfs_pread_async(nfs, fh.raw(), offset, len as u64, done_read, pcb as *mut c_void)
    });
    if reply.status < 0 {
        return Err(reply.status);
    }
    Ok(reply.status as usize)
}

/// `nfs_statvfs_async` on an already-resolved path. The caller (fs.rs) does
/// the virtual->real resolution; no RPC credentials are set here because an
/// NFS statfs is superblock-wide and path-content independent.
pub fn statvfs(path: &str) -> Result<NfsStatvfs, i32> {
    let cpath = to_cstring(path);
    let svfs = Box::into_raw(Box::new(NfsStatvfs::default()));
    let cb = SyncCbData {
        return_data: svfs as *mut c_void,
        max_size: std::mem::size_of::<NfsStatvfs>(),
        ..Default::default()
    };
    let p = pool();
    let reply = run_op(&p.slots[pick(p)], None, cb, move |nfs, pcb| unsafe {
        nfs_statvfs_async(nfs, cpath.as_ptr(), done_struct, pcb as *mut c_void)
    });
    let svfs = unsafe { *Box::from_raw(svfs) };
    if reply.status < 0 {
        return Err(reply.status);
    }
    Ok(svfs)
}
