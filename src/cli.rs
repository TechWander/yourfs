//! Minimal `getopt_long` lookalike so the Rust binaries accept the exact
//! same option set as the C version (short flags plus the long names
//! declared in the original `long_opts[]`).

use std::collections::VecDeque;

/// C `atoi()` semantics: skip leading spaces, take the longest numeric
/// prefix (`1000&gid=2000` -> 1000), garbage -> 0. A whole-string
/// `parse::<i64>()` would turn such prefixes into 0 (= root uid!), which
/// is why the URL `uid=`/`gid=` override must not use it directly.
pub fn parse_i64(s: &str) -> i64 {
    let t = s.trim_start();
    let b = t.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let digits = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if digits == i {
        return 0;
    }
    t[..i].parse::<i64>().unwrap_or(0)
}

/// C `atof()` semantics: longest parseable floating prefix (`10.5MB`
/// -> 10.5), garbage -> 0.
pub fn parse_f64(s: &str) -> f64 {
    let t = s.trim_start();
    let b = t.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let frac_start = i;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    }
    // need at least one digit before or after the dot
    if int_start == frac_start && i == frac_start {
        return 0.0;
    }
    // optional exponent, but only if it actually has digits: "1e" -> 1.0
    if i < b.len() && (b[i] | 0x20) == b'e' {
        let save = i;
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let exp_digits = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j != exp_digits {
            i = j;
        } else {
            i = save;
        }
    }
    t[..i].parse::<f64>().unwrap_or(0.0)
}

pub struct OptSpec {
    pub short: char,
    pub long: &'static str,
    pub arg: bool,
}

pub enum Command {
    /// option char, value if any
    Opt(char, Option<String>),
    /// unrecognized / malformed
    Unknown(String),
}

pub struct Parser {
    specs: &'static [OptSpec],
    args: Vec<String>,
    idx: usize,
    /// true once `--` or the first positional argument stopped options
    done: bool,
    /// no-argument flags queued inside the current cluster
    pending: Vec<Command>,
    /// commands ready to be returned (cluster flush)
    queue: VecDeque<Command>,
}

impl Parser {
    pub fn new(specs: &'static [OptSpec], args: &[String]) -> Self {
        Parser {
            specs,
            args: args.to_vec(),
            idx: 0,
            done: false,
            pending: Vec::new(),
            queue: VecDeque::new(),
        }
    }

    fn find_short(&self, c: char) -> Option<&'static OptSpec> {
        self.specs.iter().find(|s| s.short == c)
    }

    fn find_long(&self, name: &str) -> Result<Option<&'static OptSpec>, String> {
        let exact = self.specs.iter().find(|s| s.long == name);
        if exact.is_some() {
            return Ok(exact);
        }
        // getopt_long also accepts unambiguous prefixes
        let hits: Vec<&OptSpec> = self
            .specs
            .iter()
            .filter(|s| s.long.starts_with(name) && !name.is_empty())
            .collect();
        match hits.len() {
            0 => Ok(None),
            1 => Ok(Some(hits[0])),
            _ => Err(format!("option '--{name}' is ambiguous")),
        }
    }

    pub fn next_command(&mut self) -> Option<Command> {
        if let Some(c) = self.queue.pop_front() {
            return Some(c);
        }
        if self.done || self.idx >= self.args.len() {
            return None;
        }
        let a = self.args[self.idx].clone();

        // ---- long option ----
        if let Some(rest) = a.strip_prefix("--") {
            self.idx += 1;
            if rest.is_empty() {
                self.done = true;
                return self.next_command();
            }
            let (name, inline) = match rest.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (rest, None),
            };
            return match self.find_long(name) {
                Err(msg) => Some(Command::Unknown(msg)),
                Ok(None) => Some(Command::Unknown(format!("--{name}"))),
                Ok(Some(spec)) if spec.arg => match inline {
                    Some(v) => Some(Command::Opt(spec.short, Some(v))),
                    None => {
                        if self.idx < self.args.len() {
                            let v = self.args[self.idx].clone();
                            self.idx += 1;
                            Some(Command::Opt(spec.short, Some(v)))
                        } else {
                            Some(Command::Unknown(format!("--{name} requires an argument")))
                        }
                    }
                },
                Ok(Some(spec)) => Some(Command::Opt(spec.short, None)),
            };
        }

        // ---- short option cluster like -abc or -nvalue or -n value ----
        if a.len() > 1 && a.starts_with('-') {
            let flags: Vec<char> = a[1..].chars().collect();
            let mut i = 0;
            while i < flags.len() {
                let c = flags[i];
                i += 1;
                let spec = match self.find_short(c) {
                    Some(s) => s,
                    None => return Some(Command::Unknown(format!("-{c}"))),
                };
                if !spec.arg {
                    self.pending.push(Command::Opt(c, None));
                    continue;
                }
                let val = if i < flags.len() {
                    // "-Xvalue" style: rest of the cluster is the argument
                    flags[i..].iter().collect()
                } else if self.idx + 1 < self.args.len() {
                    let v = self.args[self.idx + 1].clone();
                    self.idx += 1;
                    v
                } else {
                    return Some(Command::Unknown(format!("-{c} requires an argument")));
                };
                // flush the no-arg flags queued before this one, then return it
                let take = std::mem::take(&mut self.pending);
                self.queue.extend(take);
                self.idx += 1;
                self.queue.push_back(Command::Opt(c, Some(val)));
                return Some(self.queue.pop_front().unwrap());
            }
            self.idx += 1;
            let take = std::mem::take(&mut self.pending);
            self.queue.extend(take);
            return self.next_command();
        }

        // ---- first positional argument ends option processing (glibc default) ----
        self.done = true;
        self.next_command()
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_f64, parse_i64};

    #[test]
    fn atoi_prefix_semantics() {
        assert_eq!(parse_i64("1000"), 1000);
        assert_eq!(parse_i64("  42x"), 42);
        assert_eq!(parse_i64("1000&gid=2000"), 1000);
        assert_eq!(parse_i64("-7abc"), -7);
        assert_eq!(parse_i64("abc"), 0);
        assert_eq!(parse_i64(""), 0);
        assert_eq!(parse_i64("+"), 0);
        assert_eq!(parse_i64("99999999999999999999999"), 0); // overflow -> like atoi's garbage, clamped to 0 via parse fail
    }

    #[test]
    fn atof_prefix_semantics() {
        assert_eq!(parse_f64("10.5"), 10.5);
        assert_eq!(parse_f64("10.5MB"), 10.5);
        assert_eq!(parse_f64(".5x"), 0.5);
        assert_eq!(parse_f64("1e3"), 1000.0);
        assert_eq!(parse_f64("1e"), 1.0);
        assert_eq!(parse_f64("xyz"), 0.0);
        assert_eq!(parse_f64("-"), 0.0);
    }
}

/// The `long_opts[]` + optstring of the C `yourfs` binary.
pub const YOURFS_SPECS: &[OptSpec] = &[
    OptSpec { short: '?', long: "help", arg: false },
    OptSpec { short: 'n', long: "nfs_share", arg: true },
    OptSpec { short: 'm', long: "mountpoint", arg: true },
    OptSpec { short: 'U', long: "yourfs_uid", arg: true },
    OptSpec { short: 'G', long: "yourfs_gid", arg: true },
    OptSpec { short: 'o', long: "yourfs_allow_other_own_ids", arg: false },
    OptSpec { short: 'a', long: "allow_other", arg: false },
    OptSpec { short: 'u', long: "uid", arg: true },
    OptSpec { short: 'g', long: "gid", arg: true },
    OptSpec { short: 'D', long: "debug", arg: false },
    OptSpec { short: 'p', long: "default_permissions", arg: true },
    OptSpec { short: 'd', long: "direct_io", arg: false },
    OptSpec { short: 'r', long: "allow_root", arg: false },
    OptSpec { short: 'k', long: "kernel_cache", arg: false },
    OptSpec { short: 'c', long: "auto_cache", arg: false },
    OptSpec { short: 'l', long: "large_read", arg: false },
    OptSpec { short: 'L', long: "logfile", arg: true },
    OptSpec { short: 'h', long: "hard_remove", arg: false },
    OptSpec { short: 'f', long: "fsname", arg: true },
    OptSpec { short: 's', long: "subtype", arg: true },
    OptSpec { short: 'b', long: "blkdev", arg: false },
    OptSpec { short: 'i', long: "intr", arg: false },
    OptSpec { short: 'R', long: "max_read", arg: true },
    OptSpec { short: 'H', long: "max_readahead", arg: true },
    OptSpec { short: 'A', long: "async_read", arg: false },
    OptSpec { short: 'S', long: "sync_read", arg: false },
    OptSpec { short: 'K', long: "umask", arg: true },
    OptSpec { short: 'E', long: "entry_timeout", arg: true },
    OptSpec { short: 'N', long: "negative_timeout", arg: true },
    OptSpec { short: 'T', long: "attr_timeout", arg: true },
    OptSpec { short: 'C', long: "ac_attr_timeout", arg: true },
    OptSpec { short: 'Y', long: "nonempty", arg: false },
    OptSpec { short: 'I', long: "intr_signal", arg: true },
    OptSpec { short: 'q', long: "use_ino", arg: false },
    OptSpec { short: 'Q', long: "readdir_ino", arg: false },
    OptSpec { short: 't', long: "multithread", arg: true },
    OptSpec { short: 'O', long: "read_only", arg: false },
    OptSpec { short: 'X', long: "database", arg: true },
    OptSpec { short: 'Z', long: "table", arg: true },
    OptSpec { short: 'M', long: "cache_memory", arg: true },
    OptSpec { short: 'w', long: "whitelist_cache", arg: false },
    OptSpec { short: 'V', long: "nfs_connections", arg: true },
    OptSpec { short: 'F', long: "filter_database", arg: true },
    OptSpec { short: 'P', long: "passwd", arg: true },
    OptSpec { short: 'y', long: "yourkey", arg: true },
];

/// The `long_opts[]` of the C `yourfs_tool` binary (optstring
/// `"?s:Do:X:Z:M:F:i:P:"`).
pub const TOOL_SPECS: &[OptSpec] = &[
    OptSpec { short: '?', long: "help", arg: false },
    OptSpec { short: 's', long: "scan", arg: true },
    OptSpec { short: 'i', long: "input", arg: true },
    OptSpec { short: 'o', long: "output", arg: true },
    OptSpec { short: 'X', long: "database", arg: true },
    OptSpec { short: 'Z', long: "table", arg: true },
    OptSpec { short: 'M', long: "cache_memory", arg: true },
    OptSpec { short: 'F', long: "filter_database", arg: true },
    OptSpec { short: 'P', long: "passwd", arg: true },
];
