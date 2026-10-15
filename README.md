# yourfs

**English** | [简体中文](README.zh-CN.md)

A **read-only file system with file-level isolation over NFS**, written in Rust.

`yourfs` mounts a remote NFS export through [libnfs] and exposes to the user a
*virtual* directory tree that is defined entirely by a **whitelist** stored in a
database (SQLite3, PostgreSQL or Redis). Anything not listed in the whitelist is
simply invisible; a listed entry can also be shown under a different name than it
has on the server. The result is a read-only view where you control, per file,
exactly what is exposed and under what name.

> This is a Rust rewrite of the original C project
> [TechWander/yourfs](https://github.com/TechWander/yourfs). The behaviour of the
> C version (including its quirks and its license-key gate) is preserved.

---

## How it works

The virtual tree is described by one table of triples:

| column     | meaning                                                        |
|------------|----------------------------------------------------------------|
| `parent`   | the virtual directory the entry lives in (`/` = root)          |
| `name`     | the name shown to the user                                     |
| `realpath` | the real path of the file on the NFS export                    |

```
            user sees            whitelist row            reads via libnfs
   /mnt ─── a.txt   ───────►  (/, a.txt, /a.txt)  ───────►  nfs://host/export/a.txt
            (b.txt is absent from the table, so it does not exist here)
```

* **`readdir`** lists the children of a directory (`WHERE parent = ? ORDER BY name`).
* **`lookup`/`open`/`getattr`** resolve a virtual path to its `realpath` and fetch
  it over NFS. A path with no matching row returns `ENOENT`.
* The mount is **read-only**: any write returns `EROFS`.

Two binaries are produced:

* **`yourfs`** — the FUSE file system; mounts a whitelist-defined view of an NFS export.
* **`yourfs-tool`** — builds the whitelist: scans a local tree into a text file and
  loads it into SQLite3 / PostgreSQL / Redis.

## Backends and `-X` connection strings

| backend    | `-F` value  | `-X` format                                                |
|------------|-------------|------------------------------------------------------------|
| SQLite3    | `sqlite3`   | `/path/to/data.db`                                         |
| PostgreSQL | `postgres`  | `ip,port,user,passwd,dbname`                               |
| Redis      | `redis`     | `redis://host:port/db` · `ip,port[,db[,passwd]]` · `host:port` |

For Redis the whitelist is kept as, per directory: a sorted set `{table}|d|{parent}`
(member = child name, giving byte-ordered `readdir`) and a hash `{table}|h|{parent}`
(`name → realpath`, giving `O(1)` resolution).

## Build

```bash
cargo build --release
```

This produces `target/release/yourfs` and `target/release/yourfs-tool`. The build
statically embeds `libnfs` and `libfuse` (see [`build.rs`](build.rs)), so **you do
not need to install libnfs/libfuse packages** to run the result — the only dynamic
dependencies left are the glibc base libraries, present on every Linux.

Set `YOURFS_DYNAMIC=1` to force dynamic linking, or `YOURFS_LIB_PATH=<dir>` to point
at specific `.a` files.

### Fully static (musl) build — runs on musl/Alpine too

```bash
./scripts/build-musl-static.sh
# -> target/x86_64-unknown-linux-musl/release/yourfs{,-tool}
```

The script rebuilds libfuse and libnfs against musl and links everything into a
single ELF with **zero dynamic dependencies** (`ldd` reports *statically linked*).
See [Deployment](#deployment-and-runtime-requirements) for what the target machine
still needs for FUSE itself.

## Quick start

```bash
# 1. Scan the export tree into a whitelist text file (parent name realpath per line)
yourfs-tool -s /srv/nfs/export -o list.txt

# 2. Load it into a SQLite3 database, table "data"
yourfs-tool -F sqlite3 -X /srv/data.db -Z data -i list.txt

# 3. Mount a read-only, isolated view of the NFS export
mkdir -p /mnt/yourfs
yourfs -n nfs://127.0.0.1/srv/nfs/export -m /mnt/yourfs \
       -F sqlite3 -X /srv/data.db -Z data -y FADEP8

ls /mnt/yourfs        # only whitelisted entries are visible
umount /mnt/yourfs    # or: fusermount -u /mnt/yourfs
```

## Options

**`yourfs`**

| option | meaning |
|--------|---------|
| `-n <url>` | NFS export to mount, e.g. `nfs://host/path` (**required**) |
| `-m <path>` | local mount point (**required**, implied read-only) |
| `-F <type>` | whitelist backend: `sqlite3` \| `postgres` \| `redis` |
| `-X <db>` | backend connection string (see table above) |
| `-Z <table>` | whitelist table / key prefix |
| `-M <MB>` | SQLite page cache size (default 10) |
| `-w` | cache the whitelist in memory (opt-in; enable only if it will not change during the mount) |
| `-V <N>` | number of parallel NFS connections to open (default 4; `1` disables NFS pooling) |
| `-y <key>` | license key (see [License key](#license-key)) |
| `-?` | usage |

Plus a set of libfuse pass-through flags mirroring the C version:
`-a` allow_other, `-d` direct_io, `-k` kernel_cache, `-c` auto_cache,
`-A`/`-S` async/sync read, `-Y` nonempty, `-o` allow_other+own ids, `-p` toggle
default_permissions, `-t` toggle multithreading, `-u/-g` uid/gid, timeouts, etc.

**`yourfs-tool`**

| option | meaning |
|--------|---------|
| `-s <path>` | scan this tree into `-o` output file; if omitted, the tool writes the DB |
| `-o <file>` | output text file when scanning (default `output.txt`) |
| `-i <file>` | input text file when writing the DB (default `input.txt`) |
| `-F`/`-X`/`-Z`/`-M` | same as above (write target) |
| `-?` | usage |

## Deployment and runtime requirements

Static packaging removes the need to install `libnfs`/`libfuse`, but running **any**
FUSE file system still requires, on the target machine:

* the **`fuse` kernel module** loaded and a **`/dev/fuse`** device;
* the **`fusermount`** helper (from the `fuse` / `fuse3` package) for non-root
  mounts — this is a separate setuid program that libfuse execs and cannot be
  bundled into the `yourfs` binary.

Beyond that:

* the default (`x86_64-unknown-linux-gnu`) binary needs **glibc ≥ 2.30** and runs
  on mainstream glibc distros;
* the **musl** binary is fully static and runs on musl/Alpine as well;
* network reachability to the NFS server is required at runtime.

## License key

Like the original C project, `yourfs` gates mounting behind a built-in license
key (`-y`). A handful of keys are compiled in with expiry dates; the default `FADEP8`
is valid until 2027. This is a legacy feature carried over verbatim and is *not*
real access control — the whitelist database is.

## Testing note

`yourfs` interacts with FUSE, the kernel and a live NFS server, so a unit test
alone proves very little. The end-to-end behaviour (whitelist isolation, symlink
resolution, `EROFS` on write, `statfs`, clean unmount) was verified on a real
local NFS export for both the SQLite3 and Redis backends, on the glibc **and**
the fully static musl release binaries.

## Design notes

* **FFI, not pure-Rust crates.** `yourfs` binds the real `libfuse` (2.9 high-level
  API) and `libnfs` by hand rather than using `fuser`/`nfs-rs`. This keeps exact
  behavioural parity with the C original and lets both libraries be statically
  embedded. The bindings live in [`src/libnfs.rs`](src/libnfs.rs) and
  [`src/fuse2.rs`](src/fuse2.rs).
* **Concurrency without staleness.** libfuse's high-level loop dispatches callbacks
  on several worker threads (it releases its internal lock before calling into the
  file system). Each backend (SQLite3 / PostgreSQL / Redis) therefore checks out a
  connection from a small fixed-size pool instead of queueing on a single mutex.
  Nothing is cached by default: every operation reads the live database, so a row
  added, edited or revoked externally takes effect immediately.
* **Parallel NFS too.** `libnfs` contexts are single-connection and not
  thread-safe, so instead of one mutex-guarded context the client keeps a small
  pool of independently mounted contexts, each behind its own lock; an operation
  runs on whichever context is idle. Because `nfs_open` returns a handle bound
  to the context that created it, `pread`/`close` are routed back to that same
  context (tracked in an internal map). The lock order is strictly map-then-slot
  or slot-released-then-map — never nested — so it stays deadlock-free. The number
  of contexts is tunable with `-V <N>` (default 4; `-V 1` reverts to a single
  connection).
* **Opt-in whitelist cache (`-w`).** When enabled, resolved paths and directory
  listings are memoized so repeat operations skip the database round trip — a real
  latency win for the networked backends. It is **off by default** because a revoked
  row or an edited `realpath` would keep being served from the cache; turn it on only
  when the whitelist is guaranteed static for the lifetime of the mount.
* **A release-only FFI pitfall** was fixed during the rewrite: the NFS reply-wait
  loop polls a `struct` field that a C callback rewrites through a raw pointer.
  Under `release`, LLVM was free to hoist the load out of the loop (the `&T`
  aliasing assumption), causing a hang. The loop now reads through
  `std::ptr::read_volatile`. See [`src/nfscli.rs`](src/nfscli.rs).

## License

GPL-2.0-or-later (same as the original C project).

[libnfs]: https://github.com/sahlberg/libnfs
