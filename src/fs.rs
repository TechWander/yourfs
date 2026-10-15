//! The FUSE high-level operations, a direct port of `yourfs.c`:
//! getattr/open/read/readdir/readlink/release/statfs, everything else
//! left NULL so the kernel sees a read-only file system (plus `-oro`).

use std::sync::OnceLock;

use libc::{c_char, c_int, c_void, off_t, size_t};

use crate::db::Database;
use crate::fuse2::*;
use crate::nfscli as nfs;
use crate::paths;

/// Whitelist handle shared by all fuse callbacks via
/// `fuse_context.private_data`.
pub struct DbCtx {
    pub db: Box<dyn Database>,
    pub table: String,
}

/// Raw pointer to the static `DbCtx`, published by `run()`.
pub struct DbPtr(pub *const DbCtx);
unsafe impl Sync for DbPtr {}
unsafe impl Send for DbPtr {}

static DB: OnceLock<DbPtr> = OnceLock::new();
static CUSTOM_UID: OnceLock<i32> = OnceLock::new();
static CUSTOM_GID: OnceLock<i32> = OnceLock::new();

fn db() -> &'static DbCtx {
    unsafe { &*DB.get().map(|p| p.0).expect("fs not initialized") }
}

fn caller() -> (u32, u32) {
    let ctx = unsafe { &*fuse_get_context() };
    (ctx.uid, ctx.gid)
}

/// `map_uid` / `map_gid` from the C version.
fn map_uid(possible: u32) -> u32 {
    let cu = *CUSTOM_UID.get_or_init(|| -1);
    if cu != -1 && possible == cu as u32 {
        caller().0
    } else {
        possible
    }
}

fn map_gid(possible: u32) -> u32 {
    let cg = *CUSTOM_GID.get_or_init(|| -1);
    if cg != -1 && possible == cg as u32 {
        caller().1
    } else {
        possible
    }
}

unsafe extern "C" fn yourfs_getattr(path: *const c_char, stbuf: *mut libc::stat) -> c_int {
    let vpath = paths::from_nul(path);
    let ctx = db();
    let real = if vpath == "/" {
        "/".to_string()
    } else {
        match ctx.db.resolve(&ctx.table, &vpath) {
            Ok(Some(r)) => r,
            Ok(None) => return -libc::ENOENT,
            Err(_) => return -libc::EIO,
        }
    };

    let st = match nfs::lstat64(&real, caller()) {
        Ok(st) => st,
        Err(e) => return e,
    };

    // libfuse hands us a `struct stat *`; write through our own ABI-identical
    // mirror so the time fields do not depend on the `libc` field spelling.
    let s = &mut *(stbuf as *mut Stat);
    *s = Stat::default();
    s.st_dev = st.nfs_dev;
    s.st_ino = st.nfs_ino;
    s.st_nlink = st.nfs_nlink;
    s.st_mode = st.nfs_mode as u32;
    s.st_uid = map_uid(st.nfs_uid as u32);
    s.st_gid = map_gid(st.nfs_gid as u32);
    s.st_rdev = st.nfs_rdev;
    s.st_size = st.nfs_size as i64;
    s.st_blksize = st.nfs_blksize as i64;
    s.st_blocks = st.nfs_blocks as i64;
    s.st_atime = st.nfs_atime as i64;
    s.st_atime_nsec = st.nfs_atime_nsec as i64;
    s.st_mtime = st.nfs_mtime as i64;
    s.st_mtime_nsec = st.nfs_mtime_nsec as i64;
    s.st_ctime = st.nfs_ctime as i64;
    s.st_ctime_nsec = st.nfs_ctime_nsec as i64;
    0
}

unsafe extern "C" fn yourfs_readlink(path: *const c_char, buf: *mut c_char, size: size_t) -> c_int {
    let vpath = paths::from_nul(path);
    // Resolve virtual -> real like getattr/open so a renamed (isolated)
    // symlink is queried at its actual location on the export. The C version
    // passed the virtual path straight through, which broke renamed links.
    let ctx = db();
    let real = match ctx.db.resolve(&ctx.table, &vpath) {
        Ok(Some(r)) => r,
        Ok(None) => return -libc::ENOENT,
        Err(_) => return -libc::EIO,
    };
    match nfs::readlink(&real, size, caller()) {
        Ok(target) => {
            let bytes = target.as_bytes();
            let n = bytes.len().min(size);
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, n);
            if n < size {
                *buf.add(n) = 0;
            } else {
                *buf.add(size - 1) = 0;
            }
            0
        }
        Err(e) => e,
    }
}

unsafe extern "C" fn yourfs_open(path: *const c_char, fi: *mut fuse_file_info) -> c_int {
    let vpath = paths::from_nul(path);
    let ctx = db();
    let real = match ctx.db.resolve(&ctx.table, &vpath) {
        Ok(Some(r)) => r,
        Ok(None) => return -libc::ENOENT,
        Err(_) => return -libc::EIO,
    };
    (*fi).fh = 0;
    match nfs::open(&real, (*fi).flags, caller()) {
        Ok(nfsfh) => {
            (*fi).fh = nfsfh as u64;
            0
        }
        Err(e) => e,
    }
}

unsafe extern "C" fn yourfs_read(
    _path: *const c_char,
    buf: *mut c_char,
    size: size_t,
    offset: off_t,
    fi: *mut fuse_file_info,
) -> c_int {
    let nfsfh = (*fi).fh as *mut crate::libnfs::nfsfh;
    let slice = std::slice::from_raw_parts_mut(buf as *mut u8, size);
    match nfs::pread(nfsfh, offset as u64, slice, caller()) {
        Ok(n) => n as c_int,
        Err(e) => e,
    }
}

unsafe extern "C" fn yourfs_release(_path: *const c_char, fi: *mut fuse_file_info) -> c_int {
    let nfsfh = (*fi).fh as *mut crate::libnfs::nfsfh;
    if !nfsfh.is_null() {
        nfs::close(nfsfh);
    }
    0
}

unsafe extern "C" fn yourfs_readdir(
    path: *const c_char,
    buf: *mut c_void,
    filler: FuseFillDir,
    _offset: off_t,
    _fi: *mut fuse_file_info,
) -> c_int {
    let vpath = paths::from_nul(path);
    let ctx = db();
    let names = match ctx.db.list(&ctx.table, &vpath) {
        Ok(n) => n,
        Err(_) => return -libc::EIO,
    };
    let dot = paths::to_cstring(".");
    let dotdot = paths::to_cstring("..");
    filler(buf, dot.as_ptr(), std::ptr::null(), 0);
    filler(buf, dotdot.as_ptr(), std::ptr::null(), 0);
    for name in names {
        if name.is_empty() {
            continue;
        }
        let c = paths::to_cstring(&name);
        filler(buf, c.as_ptr(), std::ptr::null(), 0);
    }
    0
}

unsafe extern "C" fn yourfs_statfs(path: *const c_char, stbuf: *mut Statvfs) -> c_int {
    let vpath = paths::from_nul(path);
    // Resolve virtual -> real like getattr/open/readlink so `df` on a renamed
    // (isolated) directory queries its actual location instead of a path that
    // does not exist on the export.
    let ctx = db();
    let real = if vpath == "/" {
        "/".to_string()
    } else {
        match ctx.db.resolve(&ctx.table, &vpath) {
            Ok(Some(r)) => r,
            Ok(None) => return -libc::ENOENT,
            Err(_) => return -libc::EIO,
        }
    };
    match nfs::statvfs(&real) {
        Ok(sv) => {
            *stbuf = sv;
            0
        }
        Err(e) => e,
    }
}

/// The `nfs_oper` table of the C version.
pub fn operations() -> fuse_operations {
    let mut op = fuse_operations::empty();
    op.getattr = Some(yourfs_getattr);
    op.readlink = Some(yourfs_readlink);
    op.open = Some(yourfs_open);
    op.read = Some(yourfs_read);
    op.release = Some(yourfs_release);
    op.readdir = Some(yourfs_readdir);
    op.statfs = Some(yourfs_statfs);
    op
}

/// Enter the fuse main loop. `argv` already carries the mountpoint at
/// index 1 and all `-o…` options, as built by the CLI layer.
pub fn run(
    ctx: &DbCtx,
    custom_uid: i32,
    custom_gid: i32,
    argv: &mut Vec<std::ffi::CString>,
) -> i32 {
    let _ = DB.set(DbPtr(ctx as *const DbCtx));
    let _ = CUSTOM_UID.set(custom_uid);
    let _ = CUSTOM_GID.set(custom_gid);

    let ops = operations();
    // fuse consumes `char **argv`; hand it leaked C strings and reclaim
    // them once the (blocking) main loop returns.
    let mut argv_ptrs: Vec<*mut c_char> =
        argv.drain(..).map(|s| s.into_raw()).collect();
    let ret = unsafe {
        fuse_main_real(
            argv_ptrs.len() as c_int,
            argv_ptrs.as_mut_ptr(),
            &ops,
            std::mem::size_of::<fuse_operations>(),
            ctx as *const DbCtx as *mut c_void,
        )
    };
    for p in argv_ptrs.drain(..) {
        drop(unsafe { std::ffi::CString::from_raw(p) });
    }
    ret
}
