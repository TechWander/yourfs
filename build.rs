use std::path::PathBuf;

/// Locate a static archive on the filesystem. We deliberately avoid probing
/// via `cc -print-file-name` because that requires spawning a child process,
/// which is unavailable in some sandboxed build environments.
///
/// For a musl target the archives must have been built against musl, so only
/// `YOURFS_LIB_PATH` is consulted (the glibc archives in the default dirs
/// would silently produce a broken binary). For gnu targets the standard
/// multiarch directories are searched, with `YOURFS_LIB_PATH` taking
/// precedence when set.
fn static_archive(name: &str, musl: bool) -> Option<PathBuf> {
    let extra: Vec<PathBuf> = std::env::var("YOURFS_LIB_PATH")
        .map(|v| v.split(':').map(PathBuf::from).collect())
        .unwrap_or_default();
    let mut dirs = extra.clone();
    if !musl {
        dirs.extend(
            [
                "/usr/lib/x86_64-linux-gnu",
                "/lib/x86_64-linux-gnu",
                "/usr/lib64",
                "/usr/lib",
                "/usr/local/lib",
                "/usr/local/lib64",
            ]
            .iter()
            .map(|d| PathBuf::from(*d)),
        );
    }
    dirs.into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=YOURFS_DYNAMIC");
    println!("cargo:rerun-if-env-changed=YOURFS_LIB_PATH");

    // Goal: ship a binary with as few runtime package requirements as
    // possible. libnfs and libfuse are the only "extra" shared libraries we
    // need, so we embed them statically. On the gnu target this leaves only
    // the glibc base libs (present on every Linux) dynamic; on the musl
    // target the whole binary, libc included, is fully static. Set
    // YOURFS_DYNAMIC=1 to force dynamic linking on the gnu target instead.
    let musl = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("musl");
    let force_dynamic = std::env::var_os("YOURFS_DYNAMIC").is_some();
    let nfs = static_archive("libnfs.a", musl);
    let fuse = static_archive("libfuse.a", musl);

    if !(force_dynamic && !musl) && nfs.is_some() && fuse.is_some() {
        // --whole-archive is REQUIRED here: libnfs registers its RPC
        // transport backends via __attribute__((constructor)) in object files
        // that are never referenced directly. Without it the linker drops
        // those objects, the constructors never run, and every NFS operation
        // hangs at runtime.
        println!("cargo:rustc-link-arg=-Wl,--whole-archive");
        println!("cargo:rustc-link-arg={}", nfs.unwrap().display());
        println!("cargo:rustc-link-arg={}", fuse.unwrap().display());
        println!("cargo:rustc-link-arg=-Wl,--no-whole-archive");
        // glibc keeps pthread/dl/m as separate libraries that libfuse/libnfs
        // reference; musl folds them all into libc, so nothing extra is
        // needed there (and -lpthread/-ldl would not even resolve).
        if musl {
            // Fully static link is single-pass, and the sysroot already placed
            // its `-lc` before our archives. Re-scan libc afterwards so the
            // symbols our --whole-archive objects pull in (perror, mount,
            // pthread_getspecific, vmsplice, ...) get resolved.
            println!("cargo:rustc-link-arg=-lc");
        } else {
            println!("cargo:rustc-link-arg=-lpthread");
            println!("cargo:rustc-link-arg=-ldl");
            println!("cargo:rustc-link-arg=-lm");
        }
        return;
    }

    println!("cargo:warning=static libnfs/libfuse not found; linking dynamically");
    for (lib, pkg) in [("fuse", "fuse"), ("libnfs", "libnfs")] {
        if pkg_config::probe_library(pkg).is_err() {
            println!("cargo:rustc-link-arg=-l{lib}");
        }
    }
}
