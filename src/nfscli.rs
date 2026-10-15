//! Synchronous façade over the libnfs async API.
//!
//! ## Concurrency model
//! libnfs's `struct nfs_context` is a single TCP connection with its own RPC
//! state and is **not** thread-safe: two commands must not be issued on the
//! same context concurrently. But libfuse's high-level loop dispatches
//! callbacks on several worker threads (it drops its internal lock before
//! calling in). To let those threads overlap real NFS round trips we keep a
//! small pool of independently mounted contexts, each guarded by its own
//! mutex, and hand an operation to whichever context is free (`acquire`).
//! Path-only operations (lstat / readlink / statvfs) can run on any context,
//! so different threads hit different sockets in parallel.
//!
//! ## File handles are pinned to a context
//! `nfs_open` returns an `nfsfh` that belongs to the context which created it
//! (it carries that connection's RPC state), so `pread`/`close` must be issued
//! on the *same* context, not just any free one. We record the owning slot in
//! `open_files` and route those calls back to it.
//!
//! ## Lock discipline (this is what keeps it deadlock-free)
//! The original C code interleaved two mutexes with unlock-after-issue /
//! lock-after-reply (and a couple of unbalanced unlocks). Here an operation
//! holds exactly one slot lock for its whole issue+wait span, and the
//! `open_files` map lock is always taken *before* the slot lock (pread/close)
//! or *after* releasing it (open) — never nested. So there is no lock cycle
//! and no way to deadlock, while still allowing cross-context parallelism.
//!
//! Replies are awaited by polling `nfs_service()` on the held context; the
//! wait loop reads the callback struct through `read_volatile` (see the note
//! on [`wait_for_reply`]) because a C callback rewrites those fields.

use std::collections::HashMap;
use std::ffi::CStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

use crate::libnfs::*;
use crate::paths::to_cstring;

/// Default number of NFS connections when `-V/--nfs_connections` is not given.
/// Acts as the upper bound: if the server refuses the extra mounts we simply
/// run with fewer (at least one).
pub const DEFAULT_POOL_SIZE: usize = 4;

/// One mounted context. `Send` so it can live behind its own mutex; the raw
/// pointer is only ever dereferenced while that mutex is held.
struct Slot {
    ctx: *mut nfs_context,
}
unsafe impl Send for Slot {}

/// Immutable, shared RPC credential configuration (identical for every slot).
struct Cfg {
    custom_uid: i32,
    custom_gid: i32,
    allow_other_own_ids: bool,
    mount_user_uid: u32,
    mount_user_gid: u32,
}

pub struct NfsPool {
    slots: Vec<Mutex<Slot>>,
    cfg: Cfg,
    /// round-robin cursor for picking a context
    next: AtomicUsize,
    /// `nfsfh` address -> index of the slot (context) that opened it
    open_files: Mutex<HashMap<usize, usize>>,
}

// A raw `*mut nfs_context` is only touched while its slot's mutex is held, so
// sharing `NfsPool` across threads is sound; the compiler cannot see through
// the raw pointer to the per-slot mutex invariant, hence this unsafe impl.
unsafe impl Sync for NfsPool {}

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
/// `pool_size == 1` keeps the original single-connection behavior.
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
    let mut slots = vec![Mutex::new(Slot { ctx: first_ctx })];
    for _ in 1..pool_size.max(1) {
        match mount_extra(server, export) {
            Some(ctx) => slots.push(Mutex::new(Slot { ctx })),
            None => break, // server limit / transient error: run with fewer
        }
    }
    let _ = NFS.set(NfsPool {
        slots,
        cfg: Cfg {
            custom_uid,
            custom_gid,
            allow_other_own_ids,
            mount_user_uid,
            mount_user_gid,
        },
        next: AtomicUsize::new(0),
        open_files: Mutex::new(HashMap::new()),
    });
}

/// Tear down every pooled context, freeing the connections opened by `init`
/// (the first caller-supplied one plus each extra mount). Call exactly once
/// after the FUSE main loop returns, when no callback can touch the pool any
/// more. Each context is destroyed while holding its own slot lock so nothing
/// can be mid-operation on it; the pointer is then nulled so a stray later
/// call cannot double-free. This also means slot 0 (the caller's original
/// context) is owned by the pool from `init` on and must NOT be destroyed
/// separately.
pub fn shutdown() {
    let Some(p) = NFS.get() else { return };
    for slot in &p.slots {
        let mut s = match slot.lock() {
            Ok(g) => g,
            // A poisoned slot lock means a callback panicked while holding it;
            // we still own the context pointer and must free it.
            Err(e) => e.into_inner(),
        };
        if !s.ctx.is_null() {
            unsafe { nfs_destroy_context(s.ctx) };
            s.ctx = std::ptr::null_mut();
        }
    }
    if let Ok(mut m) = p.open_files.lock() {
        m.clear();
    }
}

/// Borrow a context, preferring one that is already idle. Starts at a rotating
/// index and `try_lock`s each slot; if every context is busy it blocks on the
/// round-robin pick so no thread starves. Returns the slot index (so handle
/// operations can remember which context they ran on) and its guard.
fn acquire(p: &NfsPool) -> (usize, MutexGuard<'_, Slot>) {
    let n = p.slots.len();
    let start = p.next.fetch_add(1, Ordering::Relaxed) % n;
    for k in 0..n {
        let i = (start + k) % n;
        if let Ok(g) = p.slots[i].try_lock() {
            return (i, g);
        }
    }
    let i = start % n;
    (i, p.slots[i].lock().unwrap())
}

/// `update_rpc_credentials()` — call with the slot's lock held.
fn update_rpc_credentials(ctx: *mut nfs_context, cfg: &Cfg, caller_uid: u32, caller_gid: u32) {
    unsafe {
        if cfg.custom_uid == -1 && !cfg.allow_other_own_ids {
            nfs_set_uid(ctx, caller_uid as i32);
        } else if (cfg.custom_uid == -1 || caller_uid != cfg.mount_user_uid)
            && cfg.allow_other_own_ids
        {
            nfs_set_uid(ctx, caller_uid as i32);
        } else {
            nfs_set_uid(ctx, cfg.custom_uid);
        }

        if cfg.custom_gid == -1 && !cfg.allow_other_own_ids {
            nfs_set_gid(ctx, caller_gid as i32);
        } else if (cfg.custom_gid == -1 || caller_gid != cfg.mount_user_gid)
            && cfg.allow_other_own_ids
        {
            nfs_set_gid(ctx, caller_gid as i32);
        } else {
            nfs_set_gid(ctx, cfg.custom_gid);
        }
    }
}

/// `wait_for_reply()` — the slot's lock must already be held.
///
/// `cb` is a raw pointer, not a reference: the fields are written by the C
/// callback (`generic_cb`/`copy_struct_cb`) through the `private_data` we
/// handed libnfs, which Rust's type system cannot see mutating a `&`/`&mut`.
/// Reading them with `read_volatile` stops the optimiser from hoisting the
/// `is_finished` load out of the loop — which is exactly what made release
/// builds spin forever while debug builds happened to re-read each pass.
fn wait_for_reply(ctx: *mut nfs_context, cb: *mut SyncCbData) -> i32 {
    unsafe {
        while std::ptr::read_volatile(&(*cb).is_finished) == 0 {
            let mut pfd: libc::pollfd = std::mem::zeroed();
            pfd.fd = nfs_get_fd(ctx);
            pfd.events = nfs_which_events(ctx) as i16;
            let ret = libc::poll(&mut pfd, 1, 100);
            let revents = if ret < 0 { -1i16 } else { pfd.revents };
            if nfs_service(ctx, revents as i32) < 0 {
                return -libc::EIO;
            }
        }
        std::ptr::read_volatile(&(*cb).status)
    }
}

/// Issue one async libnfs call on any idle context and wait for its reply.
/// Used for the path-only operations that are not tied to an open file.
/// `cb`'s address must stay stable for the whole operation.
fn run_any(
    caller: (u32, u32),
    set_credentials: bool,
    cb: &mut SyncCbData,
    issue: impl FnOnce(*mut nfs_context, *mut SyncCbData) -> i32,
) -> i32 {
    let p = pool();
    let (_i, slot) = acquire(p);
    let ctx = slot.ctx;
    if set_credentials {
        update_rpc_credentials(ctx, &p.cfg, caller.0, caller.1);
    }
    let cbp: *mut SyncCbData = cb;
    let ret = issue(ctx, cbp);
    if ret < 0 {
        return ret;
    }
    wait_for_reply(ctx, cbp)
}

pub fn lstat64(path: &str, caller: (u32, u32)) -> Result<nfs_stat_64, i32> {
    let cpath = to_cstring(path);
    let st = Box::into_raw(Box::new(nfs_stat_64::default()));
    let mut cb = SyncCbData {
        return_data: st as *mut libc::c_void,
        max_size: std::mem::size_of::<nfs_stat_64>(),
        ..Default::default()
    };
    let status = run_any(caller, true, &mut cb, |nfs, pcb| unsafe {
        nfs_lstat64_async(nfs, cpath.as_ptr(), copy_struct_cb, pcb as *mut libc::c_void)
    });
    let st = unsafe { *Box::from_raw(st) };
    if status < 0 {
        return Err(status);
    }
    Ok(st)
}

pub fn readlink(path: &str, size: usize, caller: (u32, u32)) -> Result<String, i32> {
    let cpath = to_cstring(path);
    let mut buf = Box::new(vec![0u8; size + 1]);
    let mut cb = SyncCbData {
        return_data: buf.as_mut_ptr() as *mut libc::c_void,
        max_size: size,
        ..Default::default()
    };
    let status = run_any(caller, true, &mut cb, |nfs, pcb| unsafe {
        nfs_readlink_async(nfs, cpath.as_ptr(), readlink_cb, pcb as *mut libc::c_void)
    });
    if status < 0 {
        return Err(status);
    }
    let n = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    Ok(String::from_utf8_lossy(&buf[..n]).into_owned())
}

/// Open on a chosen context and remember which slot owns the handle, so later
/// reads/closes route back to the same connection. The slot lock is released
/// before the `open_files` lock is taken (no nesting).
///
/// Returns the opaque nfsfh pointer to store in `fuse_file_info.fh`.
pub fn open(path: &str, flags: i32, caller: (u32, u32)) -> Result<*mut nfsfh, i32> {
    let cpath = to_cstring(path);
    let mut cb = SyncCbData::default();
    let p = pool();
    let (i, slot) = acquire(p);
    let ctx = slot.ctx;
    update_rpc_credentials(ctx, &p.cfg, caller.0, caller.1);
    let cbp: *mut SyncCbData = &mut cb;
    let ret = unsafe { nfs_open_async(ctx, cpath.as_ptr(), flags, open_cb, cbp as *mut libc::c_void) };
    if ret < 0 {
        return Err(ret);
    }
    let status = wait_for_reply(ctx, cbp);
    drop(slot); // free the context before touching the map — lock discipline
    if status < 0 {
        return Err(status);
    }
    let rd = unsafe { std::ptr::read_volatile(&cb.return_data) };
    let fh = rd as *mut nfsfh;
    p.open_files.lock().unwrap().insert(fh as usize, i);
    Ok(fh)
}

/// Close using the context that opened the handle. Map lookup is released
/// before the slot lock is taken.
pub fn close(nfsfh: *mut nfsfh) {
    let p = pool();
    let slot_idx = p.open_files.lock().unwrap().remove(&(nfsfh as usize));
    let Some(i) = slot_idx else { return };
    let slot = p.slots[i].lock().unwrap();
    let ctx = slot.ctx;
    let mut cb = SyncCbData::default();
    let cbp: *mut SyncCbData = &mut cb;
    let ret = unsafe { nfs_close_async(ctx, nfsfh, generic_cb, cbp as *mut libc::c_void) };
    if ret < 0 {
        return;
    }
    let _ = wait_for_reply(ctx, cbp);
}

/// Returns the number of bytes read into `buf`, or a negative errno.
/// `buf` must outlive the call (FUSE's read buffer does). Reads must run on
/// the context that owns `nfsfh`, so we look that slot up first (briefly
/// holding only the map lock), then drop it and take the slot lock.
pub fn pread(
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
    let slot = p.slots[i].lock().unwrap();
    let ctx = slot.ctx;
    update_rpc_credentials(ctx, &p.cfg, caller.0, caller.1);
    let mut cb = SyncCbData {
        return_data: buf.as_mut_ptr() as *mut libc::c_void,
        max_size: buf.len(),
        ..Default::default()
    };
    let cbp: *mut SyncCbData = &mut cb;
    let ret = unsafe {
        nfs_pread_async(
            ctx,
            nfsfh,
            offset,
            buf.len() as u64,
            read_cb,
            cbp as *mut libc::c_void,
        )
    };
    if ret < 0 {
        return Err(ret);
    }
    let status = wait_for_reply(ctx, cbp);
    if status < 0 {
        return Err(status);
    }
    Ok(status as usize)
}

/// `nfs_statvfs_async` on an already-resolved path. The caller (fs.rs) does
/// the virtual->real resolution; no RPC credentials are set here because an
/// NFS statfs is superblock-wide and path-content independent.
pub fn statvfs(path: &str) -> Result<NfsStatvfs, i32> {
    let cpath = to_cstring(path);
    let svfs = Box::into_raw(Box::new(NfsStatvfs::default()));
    let mut cb = SyncCbData {
        return_data: svfs as *mut libc::c_void,
        max_size: std::mem::size_of::<NfsStatvfs>(),
        ..Default::default()
    };
    let status = run_any((0, 0), false, &mut cb, |nfs, pcb| unsafe {
        nfs_statvfs_async(nfs, cpath.as_ptr(), copy_struct_cb, pcb as *mut libc::c_void)
    });
    let svfs = unsafe { *Box::from_raw(svfs) };
    if status < 0 {
        return Err(status);
    }
    Ok(svfs)
}
