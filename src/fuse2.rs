//! Hand-written FFI bindings for libfuse 2.x (high-level API,
//! `FUSE_USE_VERSION 26` compatible callbacks).
//!
//! The `fuse_operations` field order mirrors libfuse 2.9 `fuse.h` exactly:
//! 38 function pointers, one 32-bit padding word, then 6 more function
//! pointers. Unused handlers are left as NULL, which makes the library
//! return -ENOSYS/EPERM for write operations — combined with `-oro` this
//! gives the read-only semantics of the original yourfs.

#![allow(non_camel_case_types, dead_code)]

use libc::{c_char, c_int, c_long, c_void, off_t, size_t, uid_t};

pub type FuseFillDir = unsafe extern "C" fn(
    buf: *mut c_void,
    name: *const c_char,
    stbuf: *const libc::stat,
    off: off_t,
) -> c_int;

type GetattrFn = unsafe extern "C" fn(*const c_char, *mut libc::stat) -> c_int;
type ReadlinkFn = unsafe extern "C" fn(*const c_char, *mut c_char, size_t) -> c_int;
type GetdirFn = unsafe extern "C" fn(*const c_char, *mut c_void, *mut c_void) -> c_int;
type MknodFn = unsafe extern "C" fn(*const c_char, libc::mode_t, libc::dev_t) -> c_int;
type MkdirFn = unsafe extern "C" fn(*const c_char, libc::mode_t) -> c_int;
type UnlinkFn = unsafe extern "C" fn(*const c_char) -> c_int;
type RmdirFn = unsafe extern "C" fn(*const c_char) -> c_int;
type SymlinkFn = unsafe extern "C" fn(*const c_char, *const c_char) -> c_int;
type RenameFn = unsafe extern "C" fn(*const c_char, *const c_char) -> c_int;
type LinkFn = unsafe extern "C" fn(*const c_char, *const c_char) -> c_int;
type ChmodFn = unsafe extern "C" fn(*const c_char, libc::mode_t) -> c_int;
type ChownFn = unsafe extern "C" fn(*const c_char, uid_t, gid_t_c) -> c_int;
type TruncateFn = unsafe extern "C" fn(*const c_char, off_t) -> c_int;
type UtimeFn = unsafe extern "C" fn(*const c_char, *mut utimbuf) -> c_int;
type OpenFn = unsafe extern "C" fn(*const c_char, *mut fuse_file_info) -> c_int;
type ReadFn =
    unsafe extern "C" fn(*const c_char, *mut c_char, size_t, off_t, *mut fuse_file_info) -> c_int;
type WriteFn =
    unsafe extern "C" fn(*const c_char, *const c_char, size_t, off_t, *mut fuse_file_info) -> c_int;
type StatfsFn = unsafe extern "C" fn(*const c_char, *mut Statvfs) -> c_int;
type FlushFn = unsafe extern "C" fn(*const c_char, *mut fuse_file_info) -> c_int;
type ReleaseFn = unsafe extern "C" fn(*const c_char, *mut fuse_file_info) -> c_int;
type FsyncFn = unsafe extern "C" fn(*const c_char, c_int, *mut fuse_file_info) -> c_int;
type SetxattrFn =
    unsafe extern "C" fn(*const c_char, *const c_char, *const c_char, size_t, c_int) -> c_int;
type GetxattrFn = unsafe extern "C" fn(*const c_char, *const c_char, *mut c_char, size_t) -> c_int;
type ListxattrFn = unsafe extern "C" fn(*const c_char, *mut c_char, size_t) -> c_int;
type RemovexattrFn = unsafe extern "C" fn(*const c_char, *const c_char) -> c_int;
type OpendirFn = unsafe extern "C" fn(*const c_char, *mut fuse_file_info) -> c_int;
type ReaddirFn = unsafe extern "C" fn(
    *const c_char,
    *mut c_void,
    FuseFillDir,
    off_t,
    *mut fuse_file_info,
) -> c_int;
type ReleasedirFn = unsafe extern "C" fn(*const c_char, *mut fuse_file_info) -> c_int;
type FsyncdirFn = unsafe extern "C" fn(*const c_char, c_int, *mut fuse_file_info) -> c_int;
type InitFn = unsafe extern "C" fn(*mut fuse_conn_info) -> *mut c_void;
type DestroyFn = unsafe extern "C" fn(*mut c_void);
type AccessFn = unsafe extern "C" fn(*const c_char, c_int) -> c_int;
type CreateFn =
    unsafe extern "C" fn(*const c_char, libc::mode_t, *mut fuse_file_info) -> c_int;
type FtruncateFn =
    unsafe extern "C" fn(*const c_char, off_t, *mut fuse_file_info) -> c_int;
type FgetattrFn =
    unsafe extern "C" fn(*const c_char, *mut libc::stat, *mut fuse_file_info) -> c_int;
type LockFn =
    unsafe extern "C" fn(*const c_char, *mut fuse_file_info, c_int, *mut libc::flock) -> c_int;
type UtimensFn = unsafe extern "C" fn(*const c_char, *const libc::timespec) -> c_int;
type BmapFn = unsafe extern "C" fn(*const c_char, size_t, *mut u64) -> c_int;
type IoctlFn = unsafe extern "C" fn(
    *const c_char,
    c_int,
    *mut c_void,
    *mut fuse_file_info,
    u32,
    *mut c_void,
) -> c_int;
type PollFn = unsafe extern "C" fn(
    *const c_char,
    *mut fuse_file_info,
    *mut c_void,
    *mut c_int,
) -> c_int;
type WriteBufFn = unsafe extern "C" fn(
    *const c_char,
    *mut fuse_bufvec,
    off_t,
    *mut fuse_file_info,
) -> c_int;
type ReadBufFn = unsafe extern "C" fn(
    *const c_char,
    *mut *mut fuse_bufvec,
    size_t,
    off_t,
    *mut fuse_file_info,
) -> c_int;
type FlockFn =
    unsafe extern "C" fn(*const c_char, *mut fuse_file_info, c_int) -> c_int;
type FallocateFn = unsafe extern "C" fn(
    *const c_char,
    c_int,
    off_t,
    off_t,
    *mut fuse_file_info,
) -> c_int;

type gid_t_c = libc::gid_t;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct utimbuf {
    pub actime: c_long,
    pub modtime: c_long,
}

/// x86_64 glibc `struct stat`, mirrored so we can populate the time fields
/// without depending on the exact `libc` field spelling. Layout matches the
/// `struct stat *` libfuse hands to `getattr`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Stat {
    pub st_dev: u64,
    pub st_ino: u64,
    pub st_nlink: u64,
    pub st_mode: u32,
    pub st_uid: u32,
    pub st_gid: u32,
    pub __pad0: i32,
    pub st_rdev: u64,
    pub st_size: i64,
    pub st_blksize: i64,
    pub st_blocks: i64,
    pub st_atime: i64,
    pub st_atime_nsec: i64,
    pub st_mtime: i64,
    pub st_mtime_nsec: i64,
    pub st_ctime: i64,
    pub st_ctime_nsec: i64,
    pub __unused: [i64; 3],
}

/// glibc `struct statvfs` layout on Linux x86_64 (verified against the C
/// ABI: 112 bytes, no `f_type` field; `f_flag` at 72, `f_namemax` at 80).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Statvfs {
    pub f_bsize: u64,
    pub f_frsize: u64,
    pub f_blocks: u64,
    pub f_bfree: u64,
    pub f_bavail: u64,
    pub f_files: u64,
    pub f_ffree: u64,
    pub f_favail: u64,
    pub f_fsid: u64,
    pub f_flag: u64,
    pub f_namemax: u64,
    pub __f_spare: [i32; 6],
}

/// `struct fuse_file_info` (libfuse 2.9).
#[repr(C)]
pub struct fuse_file_info {
    pub flags: c_int,
    pub fh_old: libc::c_ulong,
    pub writepage: c_int,
    pub bitfields: u32, // direct_io:1 keep_cache:1 flush:1 nonseekable:1 flock_release:1 padding:27
    pub fh: u64,
    pub lock_owner: u64,
}

impl fuse_file_info {
    pub fn direct_io(&self) -> bool {
        self.bitfields & 1 != 0
    }
    pub fn set_direct_io(&mut self, v: bool) {
        if v {
            self.bitfields |= 1;
        } else {
            self.bitfields &= !1;
        }
    }
    pub fn keep_cache(&self) -> bool {
        self.bitfields & 2 != 0
    }
}

/// `struct fuse_conn_info` (subset; only first fields are stable in v2.9).
#[repr(C)]
pub struct fuse_conn_info {
    pub proto_major: u32,
    pub proto_minor: u32,
    pub max_write: u32,
    pub max_readahead: u32,
    pub capable: u32,
    pub want: u32,
    pub max_background: c_int,
    pub congestion_threshold: c_int,
    pub async_read: u32,
    pub lookups: u32,
}

/// `struct fuse_buf` / `struct fuse_bufvec` (opaque use as pointers only).
#[repr(C)]
pub struct fuse_buf {
    _private: [u8; 0],
}
#[repr(C)]
pub struct fuse_bufvec {
    _private: [u8; 0],
}

/// `struct fuse_context` returned by `fuse_get_context()`.
#[repr(C)]
pub struct fuse_context {
    pub fuse: *mut c_void,
    pub uid: uid_t,
    pub gid: gid_t_c,
    pub pid: libc::pid_t,
    pub private_data: *mut c_void,
}

/// libfuse 2.9 `struct fuse_operations` — exact field order.
#[repr(C)]
pub struct fuse_operations {
    pub getattr: Option<GetattrFn>,
    pub readlink: Option<ReadlinkFn>,
    pub getdir: Option<GetdirFn>,
    pub mknod: Option<MknodFn>,
    pub mkdir: Option<MkdirFn>,
    pub unlink: Option<UnlinkFn>,
    pub rmdir: Option<RmdirFn>,
    pub symlink: Option<SymlinkFn>,
    pub rename: Option<RenameFn>,
    pub link: Option<LinkFn>,
    pub chmod: Option<ChmodFn>,
    pub chown: Option<ChownFn>,
    pub truncate: Option<TruncateFn>,
    pub utime: Option<UtimeFn>,
    pub open: Option<OpenFn>,
    pub read: Option<ReadFn>,
    pub write: Option<WriteFn>,
    pub statfs: Option<StatfsFn>,
    pub flush: Option<FlushFn>,
    pub release: Option<ReleaseFn>,
    pub fsync: Option<FsyncFn>,
    pub setxattr: Option<SetxattrFn>,
    pub getxattr: Option<GetxattrFn>,
    pub listxattr: Option<ListxattrFn>,
    pub removexattr: Option<RemovexattrFn>,
    pub opendir: Option<OpendirFn>,
    pub readdir: Option<ReaddirFn>,
    pub releasedir: Option<ReleasedirFn>,
    pub fsyncdir: Option<FsyncdirFn>,
    pub init: Option<InitFn>,
    pub destroy: Option<DestroyFn>,
    pub access: Option<AccessFn>,
    pub create: Option<CreateFn>,
    pub ftruncate: Option<FtruncateFn>,
    pub fgetattr: Option<FgetattrFn>,
    pub lock: Option<LockFn>,
    pub utimens: Option<UtimensFn>,
    pub bmap: Option<BmapFn>,
    pub padding: u32,
    pub ioctl: Option<IoctlFn>,
    pub poll: Option<PollFn>,
    pub write_buf: Option<WriteBufFn>,
    pub read_buf: Option<ReadBufFn>,
    pub flock: Option<FlockFn>,
    pub fallocate: Option<FallocateFn>,
}

impl fuse_operations {
    /// All handlers NULL except the ones set afterwards.
    pub const fn empty() -> Self {
        fuse_operations {
            getattr: None,
            readlink: None,
            getdir: None,
            mknod: None,
            mkdir: None,
            unlink: None,
            rmdir: None,
            symlink: None,
            rename: None,
            link: None,
            chmod: None,
            chown: None,
            truncate: None,
            utime: None,
            open: None,
            read: None,
            write: None,
            statfs: None,
            flush: None,
            release: None,
            fsync: None,
            setxattr: None,
            getxattr: None,
            listxattr: None,
            removexattr: None,
            opendir: None,
            readdir: None,
            releasedir: None,
            fsyncdir: None,
            init: None,
            destroy: None,
            access: None,
            create: None,
            ftruncate: None,
            fgetattr: None,
            lock: None,
            utimens: None,
            bmap: None,
            padding: 0,
            ioctl: None,
            poll: None,
            write_buf: None,
            read_buf: None,
            flock: None,
            fallocate: None,
        }
    }
}

extern "C" {
    pub fn fuse_main_real(
        argc: c_int,
        argv: *mut *mut c_char,
        op: *const fuse_operations,
        op_size: size_t,
        user_data: *mut c_void,
    ) -> c_int;

    pub fn fuse_get_context() -> *mut fuse_context;
}
