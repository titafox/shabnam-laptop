#!/bin/sh
# Cross-builds shabnam-filechooser on the Mac for the laptop (aarch64, glibc 2.41). Never build on
# the device. Needs: Rust (stable, with the aarch64-unknown-linux-gnu target), cargo-zigbuild, zig.
#   WORK = a scratch directory outside the repository.
#   build.sh         build -> $WORK/shabnam-filechooser
#   build.sh test    run the unit tests (command-line parsing) on the Mac
# Install on the device as /usr/local/bin/shabnam-filechooser (root, 755, via .new + mv) and make
# /usr/local/bin/zenity a link to it:   ln -sfn shabnam-filechooser /usr/local/bin/zenity
# The binary needs libc at start and opens libgtk-3.so.0 (package gtk+3) itself; no other package.
: "${WORK:?set WORK to a scratch directory outside the repository}"
set -e
cd "$(dirname "$0")"
export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-stable}"
export CARGO_TARGET_DIR="$WORK/target"
if [ "$1" = test ]; then exec cargo test --locked; fi
cargo zigbuild --locked --release --target aarch64-unknown-linux-gnu.2.41
cp "$CARGO_TARGET_DIR/aarch64-unknown-linux-gnu/release/shabnam-filechooser" "$WORK/shabnam-filechooser"
shasum -a 256 "$WORK/shabnam-filechooser"
