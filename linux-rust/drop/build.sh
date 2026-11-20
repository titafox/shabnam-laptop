#!/bin/sh
# Cross-builds 隔空传送 (shabnam-dropd, shabnam-send) on the Mac for the laptop (aarch64, glibc 2.41).
# Never build on the device. Needs: Rust (stable, with the aarch64-unknown-linux-gnu target),
# cargo-zigbuild, zig, curl, shasum, patch.
#   WORK = a scratch directory outside the repository (the third-party source is unpacked there).
#   build.sh         build both programs -> $WORK/shabnam-dropd, $WORK/shabnam-send
#   build.sh test    run this crate's unit tests on the Mac
# Install on the device as /usr/local/bin/shabnam-dropd and shabnam-send (root, 755, via .new + mv);
# they are started through ../device/shabnam-drop. The binaries need only libc, libm and libgcc_s.
#
# The protocol is not ours: it is the LocalSend project's own Rust core, the library its official
# apps are being moved onto (github.com/localsend/localsend, packages/core, Apache-2.0). Our code
# in src/ decides what is accepted and where it is written; the core does discovery, HTTPS,
# certificates and the upload/cancel requests (protocol 2.2, as the official apps speak it).
#   Pinned: commit e05d12a9c0fd4d8c0245d2ec5c3c177e7e0f759b (main, 2026-11-19; the release tag
#   v1.18.2 of 2026-08 predates the core's packages/core layout). Checksum below is of that commit's
#   GitHub source archive. Nothing of it is kept in this repository; upstream/ is a link into $WORK.
#   Used features: crypto, http, multicast, discovery — not webrtc (no signalling server is ever
#   contacted) and not the "share via link" pages. 250 crates, versions taken from upstream's own
#   Cargo.lock (our Cargo.lock started as a copy of it), built with --locked.
#   Read before building: packages/core has no build script; there is no self-update, no telemetry
#   and no download code in it. It makes outgoing connections only to devices that announced
#   themselves on the local network (and, when asked to scan, to the /24 of the given address).
#   Its rust-toolchain.toml pins 1.97.1 for upstream's lint checks; we build with the installed
#   stable (1.98.1), as ../files/build.sh does.
# patches/localsend-core-bind.patch (ours, 2 hunks): the core's HTTPS server and discovery socket
#   listen on every interface; the patch adds start_with_bind() to listen on one IPv4 address only
#   and, on Linux, binds the discovery socket to the group address so that only group traffic on
#   the joined interface is received. With it nothing listens on the cable to the Mac (usb0).
: "${WORK:?set WORK to a scratch directory outside the repository}"
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
COMMIT=e05d12a9c0fd4d8c0245d2ec5c3c177e7e0f759b
SHA256=e78b49dbf588bfd8f9478728e66a0860cd98926d4cc5b8c9aab7bb25865d7b3d
mkdir -p "$WORK" && cd "$WORK"
[ -f "localsend-$COMMIT.tar.gz" ] ||
  curl -fsSL --retry 5 -o "localsend-$COMMIT.tar.gz" "https://codeload.github.com/localsend/localsend/tar.gz/$COMMIT"
echo "$SHA256  localsend-$COMMIT.tar.gz" | shasum -a 256 -c -
if [ ! -f src/.shabnam-patched ]; then
  rm -rf src && mkdir src && tar xzf "localsend-$COMMIT.tar.gz" -C src --strip-components=1
  patch -d src -p1 < "$HERE/patches/localsend-core-bind.patch"
  touch src/.shabnam-patched
fi
ln -sfn "$WORK/src" "$HERE/upstream"
cd "$HERE"
export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-stable}"
export CARGO_TARGET_DIR="$WORK/target"
if [ "$1" = test ]; then exec cargo test --locked; fi
cargo zigbuild --locked --release --target aarch64-unknown-linux-gnu.2.41
for bin in shabnam-dropd shabnam-send; do cp "$CARGO_TARGET_DIR/aarch64-unknown-linux-gnu/release/$bin" "$WORK/$bin"; done
shasum -a 256 "$WORK/shabnam-dropd" "$WORK/shabnam-send"
