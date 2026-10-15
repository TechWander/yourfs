//! Path helpers mirroring `yfs_common.h` of the C implementation.

/// Split a virtual path into its `parent` directory and `name` component,
/// same semantics as C `get_path()`:
/// - `/a/b`   -> parent `/a`, name `b`
/// - `/a`     -> parent `/`,  name `a`
/// - `/`      -> parent `""`, name `""` (callers special-case the root)
pub fn split_path(path: &str) -> (String, String) {
    if path == "/" {
        return (String::new(), String::new());
    }
    match path.rfind('/') {
        Some(0) => ("/".to_string(), path[1..].to_string()),
        Some(i) => (path[..i].to_string(), path[i + 1..].to_string()),
        None => (String::new(), path.to_string()),
    }
}

/// Convert a byte string from FUSE (NUL-terminated) to a borrowed `str`,
/// lossy on invalid UTF-8, mirroring C's treatment of paths.
///
/// Returns `Cow::Borrowed` (no allocation) for the common valid-UTF-8 case;
/// this runs on every getattr/lookup/open/readdir, so avoiding a heap String
/// per callback matters. The borrow is valid for the lifetime of the FUSE
/// buffer, which outlives each synchronous callback.
pub fn from_nul<'a>(p: *const libc::c_char) -> std::borrow::Cow<'a, str> {
    if p.is_null() {
        return std::borrow::Cow::Borrowed("");
    }
    let cstr = unsafe { std::ffi::CStr::from_ptr(p as *const libc::c_char) };
    match cstr.to_str() {
        Ok(s) => std::borrow::Cow::Borrowed(s),
        Err(_) => std::borrow::Cow::Owned(cstr.to_string_lossy().into_owned()),
    }
}

/// Owned string to a heap-allocated NUL-terminated C string.
pub fn to_cstring(s: &str) -> std::ffi::CString {
    // Fast path: no interior NUL -> a single allocation inside CString::new.
    // (The old `s.replace('\0', "")` always allocated a scratch String first,
    // then a second time for the CString, doubling cost on every NFS op.)
    match std::ffi::CString::new(s) {
        Ok(c) => c,
        // Paths from FUSE never contain interior NUL; keep sanitize-and-retry
        // as a defensive fallback for any other caller.
        Err(_) => std::ffi::CString::new(s.replace('\0', "")).expect("no interior NUL after strip"),
    }
}

#[cfg(test)]
mod tests {
    use super::split_path;

    #[test]
    fn split_paths() {
        assert_eq!(split_path("/a/b"), ("/a".to_string(), "b".to_string()));
        assert_eq!(split_path("/a"), ("/".to_string(), "a".to_string()));
        assert_eq!(split_path("/x/y/z"), ("/x/y".to_string(), "z".to_string()));
        assert_eq!(split_path("/"), (String::new(), String::new()));
    }
}
