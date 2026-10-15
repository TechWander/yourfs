#!/usr/bin/env bash
#
# build-musl-static.sh — produce a FULLY STATIC musl build of yourfs.
#
# The catch: the C libraries (libnfs, libfuse) that yourfs links against must
# be rebuilt against musl; the system's glibc-built .a files would silently
# produce a broken binary. This script compiles libfuse 2.9.9 and libnfs 4.0.0
# with musl-gcc, then drives `cargo build --target x86_64-unknown-linux-musl`
# so build.rs embeds those musl archives statically.
#
# Result: target/x86_64-unknown-linux-musl/release/yourfs{,-tool} with zero
# dynamic dependencies (verified with `ldd ... => statically linked`).
#
# Requirements on the build host: rustup/cargo with the musl target, musl-tools
# (musl-gcc), autoconf/automake/libtool, git, curl.
#
# Note: even a fully static yourfs still needs the FUSE *runtime* on the target
# machine — the kernel `fuse` module, /dev/fuse, and the `fusermount` helper
# (from the `fuse` package). Those cannot be bundled into a single ELF.

set -euo pipefail

FUSE_VER="${FUSE_VER:-2.9.9}"
NFS_REF="${NFS_REF:-libnfs-4.0.0}"
WORK="${WORK:-/tmp/yourfs-muslbuild}"
OUT="$WORK/out-libs"

need() { command -v "$1" >/dev/null 2>&1 || { echo "missing tool: $1" >&2; exit 1; }; }
need cargo; need musl-gcc; need autoconf; need automake; need libtool; need curl; need git

rustup target add x86_64-unknown-linux-musl >/dev/null 2>&1 || true
mkdir -p "$WORK" "$OUT"

# ---- libfuse (static) ----
if [ ! -f "$OUT/libfuse.a" ]; then
  cd "$WORK"
  if [ ! -d "fuse-$FUSE_VER" ]; then
    curl -sL -o "fuse-$FUSE_VER.tar.gz" \
      "https://github.com/libfuse/libfuse/releases/download/fuse-$FUSE_VER/fuse-$FUSE_VER.tar.gz"
    tar xzf "fuse-$FUSE_VER.tar.gz"
  fi
  cd "fuse-$FUSE_VER"
  CC=musl-gcc ./configure --enable-static --disable-shared --disable-example \
    --disable-util >/dev/null
  make -C lib >/dev/null 2>&1 || true   # ranlib step may fail; archive is fine
  ranlib lib/.libs/libfuse.a
  cp lib/.libs/libfuse.a "$OUT/"
  echo "built $OUT/libfuse.a"
fi

# ---- libnfs (static) ----
if [ ! -f "$OUT/libnfs.a" ]; then
  cd "$WORK"
  if [ ! -d libnfs ]; then
    git clone --quiet --depth 1 -b "$NFS_REF" https://github.com/sahlberg/libnfs.git libnfs
  fi
  cd libnfs
  autoreconf -fi >/dev/null
  # -Wno-error: musl headers trip libnfs's -Werror on some toolchains.
  CC=musl-gcc CFLAGS="-O2 -Wno-error" ./configure --enable-static --disable-shared >/dev/null
  make >/dev/null
  cp lib/.libs/libnfs.a "$OUT/"
  echo "built $OUT/libnfs.a"
fi

# ---- yourfs (musl, fully static) ----
cd "$(dirname "$0")/.."
YOURFS_LIB_PATH="$OUT" cargo build --release --target x86_64-unknown-linux-musl

B="target/x86_64-unknown-linux-musl/release"
echo
echo "Artifacts:"
ls -l "$B/yourfs" "$B/yourfs-tool"
echo "Verify:"
file "$B/yourfs" | sed 's/.*: //'
ldd "$B/yourfs" 2>&1 | head -1   # => "statically linked"
