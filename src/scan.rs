//! Directory scanner ported from `yfs_scan.h`: walks a local tree and
//! emits one `parent / name / realpath` row per entry into the whitelist
//! text file, with the scan-root prefix stripped (so the virtual tree
//! starts at `/`).

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

/// Recursively append the rows of `dir`'s children to `out`.
/// `replace_num` is the trimmed scan-root length: the local prefix removed
/// from every emitted path. Errors on individual entries (permissions,
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
        // full local path, then strip the scan root; the slice may start
        // without '/' (e.g. root-relative paths already consumed the slash),
        // so restore the leading '/' or split_path would yield parent=""
        // and the whole whitelist row could never be resolved again.
        let local = child.to_string_lossy().into_owned();
        let mut out_path: String = if local.len() > replace_num {
            let rest = &local[replace_num..];
            if rest.starts_with('/') {
                rest.to_string()
            } else {
                format!("/{rest}")
            }
        } else {
            "/".to_string()
        };
        if out_path.is_empty() {
            out_path = "/".to_string();
        }
        let (parent, basename) = crate::paths::split_path(&out_path);
        let _ = writeln!(
            out,
            "{}  {}  {}",
            pad_field(&parent),
            pad_field(&basename),
            pad_field(&out_path)
        );
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

/// Width a clean (whitespace-free) field is padded to, mirroring the C
/// tool's `%-25s` so the byte layout of shared inputs stays familiar.
const FIELD_WIDTH: usize = 25;

/// Format one row field. Names with spaces/tabs or quotes would break the
/// whitespace-column reader, so such fields are double-quoted with `\"`
/// / `\\` escapes instead of padded; `parse_row` understands both forms.
fn pad_field(s: &str) -> String {
    if s.chars().any(|c| c.is_whitespace() || c == '"' || c == '\\') {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('"');
        for c in s.chars() {
            if c == '"' || c == '\\' {
                out.push('\\');
            }
            out.push(c);
        }
        out.push('"');
        out
    } else {
        format!("{:<w$}", s, w = FIELD_WIDTH)
    }
}

/// Split one output row into its three columns, mirroring the C reader's
/// `fscanf("%s %s %s")` for plain rows and additionally honoring the
/// quoted fields `pad_field` emits for whitespace-containing names.
/// Returns `None` when the row does not hold exactly three fields.
pub fn parse_row(line: &str) -> Option<[String; 3]> {
    let mut cols: Vec<String> = Vec::with_capacity(3);
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut has_field = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '\\' {
                if let Some(&n) = chars.peek() {
                    if n == '"' || n == '\\' || n.is_whitespace() {
                        cur.push(n);
                        chars.next();
                        continue;
                    }
                }
                cur.push('\\');
            } else if c == '"' {
                in_quotes = false;
            } else {
                cur.push(c);
            }
        } else if c == '"' {
            in_quotes = true;
            has_field = true;
        } else if c.is_whitespace() {
            if has_field {
                cols.push(std::mem::take(&mut cur));
                has_field = false;
            }
        } else {
            cur.push(c);
            has_field = true;
        }
    }
    if has_field {
        cols.push(cur);
    }
    if cols.len() == 3 {
        let mut it = cols.into_iter();
        Some([
            it.next().unwrap(),
            it.next().unwrap(),
            it.next().unwrap(),
        ])
    } else {
        None
    }
}

/// `yourfs_tool -s <path> -o <file>` entry point.
pub fn scan_to_file(scan_path: &str, output: &str) -> std::io::Result<()> {
    // A trailing slash (or scanning "/" itself) would otherwise make the
    // prefix slice eat the leading '/' of every emitted virtual path.
    let trimmed = scan_path.trim_end_matches('/');
    let replace_num = trimmed.len();
    let root = if trimmed.is_empty() { "/" } else { trimmed };
    let f = File::create(output)?;
    let mut out = BufWriter::new(f);
    walk(Path::new(root), replace_num, &mut out);
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::{pad_field, parse_row};

    #[test]
    fn plain_rows_tokenize_like_whitespace() {
        let row = format!(
            "{}  {}  {}",
            pad_field("/a"),
            pad_field("b"),
            pad_field("/a/b")
        );
        assert_eq!(
            parse_row(&row).unwrap(),
            ["/a".to_string(), "b".to_string(), "/a/b".to_string()]
        );
    }

    #[test]
    fn whitespace_names_roundtrip() {
        let row = format!(
            "{}  {}  {}",
            pad_field("/my dir"),
            pad_field("a b.txt"),
            pad_field("/my dir/a b.txt")
        );
        assert_eq!(
            parse_row(&row).unwrap(),
            [
                "/my dir".to_string(),
                "a b.txt".to_string(),
                "/my dir/a b.txt".to_string()
            ]
        );
    }

    #[test]
    fn corrupt_rows_are_rejected() {
        // old C-generated rows whose names contained whitespace split into
        // the wrong number of fields: refuse instead of mispairing columns
        assert!(parse_row("/a  b  /a/b c d").is_none());
        assert!(parse_row("only  two").is_none());
    }
}
