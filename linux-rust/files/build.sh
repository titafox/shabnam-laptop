#!/bin/sh
# Cross-builds the file manager behind 「文件」 (Favnyr) on the Mac for the laptop (aarch64, glibc 2.41).
# Never build on the device. Needs: Rust (stable, with the aarch64-unknown-linux-gnu target), cargo-zigbuild,
# zig, curl, shasum. Nothing from the source tree is kept in this repository.
#   SYSROOT = copy of the device's /usr tree. Needed from it: usr/lib/pkgconfig/{fontconfig,freetype2}.pc and
#             usr/lib/libfontconfig.so.1 + libfreetype.so.6 (copied from the device's /usr/lib), each with an
#             unversioned link next to it (libfontconfig.so, libfreetype.so).
#   WORK    = a scratch directory outside the repository.
# Result: $WORK/favnyr -> install as /usr/local/bin/favnyr on the device (root, 755, via .new + mv); it is
# started through ../device/shabnam-files. The binary needs only libc, libm and libfontconfig at run time;
# libxkbcommon and (for the unused GLES renderer) libEGL are opened by name when present.
#
# Source: github.com/GlitchAwakened/Favnyr, tag v0.3.8 = commit 44a96f744568cfff46b2513af753904b2114e05c
# (2026-11-04), GPL-3.0-or-later. Rust + Slint 1.18.1, 624 crates locked by its Cargo.lock (--locked).
# Read before building: its only build script (crates/favnyr-gui/build.rs) compiles the .slint file and
# nothing else; there is no network code in the application (no HTTP/TLS crate in the lock file; tokio is
# pulled in by a dependency only). At run time it starts other programs only on request: xdg-open for
# opening files, and — when installed — 7z, ffmpeg/ffprobe, pdftoppm, lsblk, udisksctl, gdbus.
# Its rust-toolchain.toml pins 1.97.1 for upstream's lint checks; RUSTUP_TOOLCHAIN below overrides that so
# no second toolchain is downloaded (built here with 1.98.1).
: "${SYSROOT:?set SYSROOT}" "${WORK:?set WORK}"
set -e
TAG=v0.3.8
SHA256=614ec31ad26d6c1934fd26bc0a64d9b0acbc300c9ebfad4118e27fcd1266db90   # of the tag's GitHub source archive
mkdir -p "$WORK" && cd "$WORK"
[ -f "favnyr-$TAG.tar.gz" ] ||
  curl -fsSL --retry 5 -o "favnyr-$TAG.tar.gz" "https://codeload.github.com/GlitchAwakened/Favnyr/tar.gz/refs/tags/$TAG"
echo "$SHA256  favnyr-$TAG.tar.gz" | shasum -a 256 -c -
[ -d src ] || { mkdir src && tar xzf "favnyr-$TAG.tar.gz" -C src --strip-components=1; }
cd src
export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-stable}"
export PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_SYSROOT_DIR="$SYSROOT" PKG_CONFIG_LIBDIR="$SYSROOT/usr/lib/pkgconfig"
export CARGO_TARGET_DIR="$WORK/target"
RUSTFLAGS="-L native=$SYSROOT/usr/lib" \
  cargo zigbuild --locked --release --target aarch64-unknown-linux-gnu.2.41 --bin favnyr
cp "$CARGO_TARGET_DIR/aarch64-unknown-linux-gnu/release/favnyr" "$WORK/favnyr"
shasum -a 256 "$WORK/favnyr"
