//! Directory scanner ported from `yfs_scan.h`: walks a local tree and
//! emits one `parent / name / realpath` row per entry into the whitelist
//! text file, with the scan-root prefix stripped (so the virtual tree
//! starts at `/`).

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

/// Recursively append the rows of `dir`'s children to `out`.
/// `replace_num` is `scan_path.len()`: the local prefix removed from
/// every emitted path. Errors on individual entries (permissions,
/// disappearing files) are skipped like the C version skips NULL opendir.
pub fn walk(dir: &Path, replace_num: usize, out: &mut dyn Write) {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    for ent in rd.flatten() {
        let name = ent.file_name().to_string_lossy().into_owned();
        if name == "." || name == ".." {
            continue;
        }
        let child = ent.path();
        // full local path, then strip the scan root
        let local = child.to_string_lossy().into_owned();
        let mut out_path: String = if local.len() > replace_num {
            local[replace_num..].to_string()
        } else {
            String::new()
        };
        if out_path.is_empty() {
            out_path = "/".to_string();
        }
        let (parent, basename) = crate::paths::split_path(&out_path);
        let _ = writeln!(out, "{:<25}  {:<25}  {:<25}", parent, basename, out_path);
        // d_type == DT_DIR in C; follow symlinks? C used readdir d_type
        // which does not resolve symlinks — a symlinked dir is DT_LNK and
        // treated as a file. `file_type()` mirrors that (lstat semantics).
        if let Ok(md) = ent.file_type() {
            if md.is_dir() {
                walk(&child, replace_num, out);
            }
        }
    }
}

/// `yourfs_tool -s <path> -o <file>` entry point.
pub fn scan_to_file(scan_path: &str, output: &str) -> std::io::Result<()> {
    let replace_num = scan_path.len();
    let f = File::create(output)?;
    let mut out = BufWriter::new(f);
    walk(Path::new(scan_path), replace_num, &mut out);
    out.flush()
}
