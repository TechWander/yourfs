//! Hand-written FFI bindings for libnfs 4.x (the subset yourfs uses).
//! All calls are the async variants queued onto the single shared context,
//! serviced by `nfs_service()` in a poll loop — same model as the C code.

#![allow(non_camel_case_types, dead_code)]

use libc::{c_char, c_int, c_void, size_t};

#[repr(C)]
pub struct nfs_context {
    _private: [u8; 0],
}

#[repr(C)]
pub struct nfsfh {
    _private: [u8; 0],
}

/// `struct nfs_url` from <nfsc/libnfs.h>.
#[repr(C)]
pub struct nfs_url {
    pub server: *mut c_char,
    pub path: *mut c_char,
    pub file: *mut c_char,
}

/// `struct nfs_stat_64` from <nfsc/libnfs.h>.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct nfs_stat_64 {
    pub nfs_dev: u64,
    pub nfs_ino: u64,
    pub nfs_mode: u64,
    pub nfs_nlink: u64,
    pub nfs_uid: u64,
    pub nfs_gid: u64,
    pub nfs_rdev: u64,
    pub nfs_size: u64,
    pub nfs_blksize: u64,
    pub nfs_blocks: u64,
    pub nfs_atime: u64,
    pub nfs_mtime: u64,
    pub nfs_ctime: u64,
    pub nfs_atime_nsec: u64,
    pub nfs_mtime_nsec: u64,
    pub nfs_ctime_nsec: u64,
    pub nfs_used: u64,
}

/// glibc `struct statvfs` layout (same as `fuse2::Statvfs`); libnfs's
/// `nfs_statvfs_async` callback hands us one of these.
pub type NfsStatvfs = crate::fuse2::Statvfs;

/// `typedef void (*nfs_cb)(int err, struct nfs_context *context, void *private_data, void *call_data);`
pub type NfsCb = extern "C" fn(
    status: c_int,
    nfs: *mut nfs_context,
    data: *mut c_void,
    private_data: *mut c_void,
);

extern "C" {
    pub fn nfs_init_context() -> *mut nfs_context;
    pub fn nfs_destroy_context(nfs: *mut nfs_context);
    pub fn nfs_get_error(nfs: *mut nfs_context) -> *mut c_char;

    pub fn nfs_parse_url_dir(nfs: *mut nfs_context, url: *const c_char) -> *mut nfs_url;
    pub fn nfs_destroy_url(url: *mut nfs_url);

    pub fn nfs_mount(nfs: *mut nfs_context, server: *const c_char, export: *const c_char)
        -> c_int;

    pub fn nfs_get_fd(nfs: *mut nfs_context) -> c_int;
    pub fn nfs_which_events(nfs: *mut nfs_context) -> c_int;
    pub fn nfs_service(nfs: *mut nfs_context, revents: c_int) -> c_int;

    pub fn nfs_set_uid(nfs: *mut nfs_context, uid: c_int);
    pub fn nfs_set_gid(nfs: *mut nfs_context, gid: c_int);

    pub fn nfs_lstat64_async(
        nfs: *mut nfs_context,
        path: *const c_char,
        cb: NfsCb,
        private_data: *mut c_void,
    ) -> c_int;
    pub fn nfs_readlink_async(
        nfs: *mut nfs_context,
        path: *const c_char,
        cb: NfsCb,
        private_data: *mut c_void,
    ) -> c_int;
    pub fn nfs_open_async(
        nfs: *mut nfs_context,
        path: *const c_char,
        flags: c_int,
        cb: NfsCb,
        private_data: *mut c_void,
    ) -> c_int;
    pub fn nfs_close_async(
        nfs: *mut nfs_context,
        nfsfh: *mut nfsfh,
        cb: NfsCb,
        private_data: *mut c_void,
    ) -> c_int;
    pub fn nfs_pread_async(
        nfs: *mut nfs_context,
        nfsfh: *mut nfsfh,
        offset: u64,
        count: u64,
        cb: NfsCb,
        private_data: *mut c_void,
    ) -> c_int;
    pub fn nfs_statvfs_async(
        nfs: *mut nfs_context,
        path: *const c_char,
        cb: NfsCb,
        private_data: *mut c_void,
    ) -> c_int;
}

/// Callback payload mirroring the C `struct sync_cb_data`.
#[repr(C)]
pub struct SyncCbData {
    pub is_finished: c_int,
    pub status: c_int,
    pub return_data: *mut c_void,
    pub max_size: size_t,
}

impl Default for SyncCbData {
    fn default() -> Self {
        SyncCbData {
            is_finished: 0,
            status: 0,
            return_data: std::ptr::null_mut(),
            max_size: 0,
        }
    }
}

/// Generic completion callback: just record status.
pub extern "C" fn generic_cb(
    status: c_int,
    _nfs: *mut nfs_context,
    _data: *mut c_void,
    private_data: *mut c_void,
) {
    let cb = unsafe { &mut *(private_data as *mut SyncCbData) };
    cb.is_finished = 1;
    cb.status = status;
}

/// Callback for `nfs_lstat64_async` / `nfs_statvfs_async`: copy the
/// returned struct into `return_data`.
pub extern "C" fn copy_struct_cb(
    status: c_int,
    _nfs: *mut nfs_context,
    data: *mut c_void,
    private_data: *mut c_void,
) {
    let cb = unsafe { &mut *(private_data as *mut SyncCbData) };
    cb.is_finished = 1;
    cb.status = status;
    if status < 0 || data.is_null() || cb.return_data.is_null() {
        return;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(
            data as *const u8,
            cb.return_data as *mut u8,
            cb.max_size,
        );
    }
}

/// Callback for `nfs_readlink_async`: append the NUL-terminated target.
pub extern "C" fn readlink_cb(
    status: c_int,
    _nfs: *mut nfs_context,
    data: *mut c_void,
    private_data: *mut c_void,
) {
    let cb = unsafe { &mut *(private_data as *mut SyncCbData) };
    cb.is_finished = 1;
    cb.status = status;
    if status < 0 || data.is_null() || cb.return_data.is_null() {
        return;
    }
    unsafe {
        let dst = cb.return_data as *mut c_char;
        let src = data as *const c_char;
        // strncat(dst, src, max_size) semantics, identical to the C version
        // (fuse pre-NUL-terminates `dst`, and the kernel guarantees the size).
        libc::strncat(dst, src, cb.max_size);
    }
}

/// Callback for `nfs_open_async`: store the returned `nfsfh*` pointer.
pub extern "C" fn open_cb(
    status: c_int,
    _nfs: *mut nfs_context,
    data: *mut c_void,
    private_data: *mut c_void,
) {
    let cb = unsafe { &mut *(private_data as *mut SyncCbData) };
    cb.is_finished = 1;
    cb.status = status;
    if status >= 0 {
        cb.return_data = data;
    }
}

/// Callback for `nfs_pread_async`: memcpy `status` bytes into the buffer.
pub extern "C" fn read_cb(
    status: c_int,
    _nfs: *mut nfs_context,
    data: *mut c_void,
    private_data: *mut c_void,
) {
    let cb = unsafe { &mut *(private_data as *mut SyncCbData) };
    cb.is_finished = 1;
    cb.status = status;
    if status < 0 || data.is_null() {
        return;
    }
    unsafe {
        // Bound the copy by the buffer we handed in (`max_size`), never trust
        // `status` alone: a short/over-long reply must not run past the buffer.
        let n = (status as usize).min(cb.max_size);
        std::ptr::copy_nonoverlapping(data as *const u8, cb.return_data as *mut u8, n);
        // Report the byte count we actually delivered so it matches the buffer.
        cb.status = n as c_int;
    }
}
