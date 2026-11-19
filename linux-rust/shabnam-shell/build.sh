#!/bin/sh
# Cross-builds shabnam-shell on the Mac for the laptop (aarch64, glibc 2.41). Never build on the device.
#   SYSROOT = a copy of the device's /usr tree (for libxkbcommon and its pkg-config file).
# Result: $CARGO_TARGET_DIR/aarch64-unknown-linux-gnu/release/shabnam-shell
: "${SYSROOT:?set SYSROOT to the directory holding the device sysroot (it contains usr/lib)}"
cd "$(dirname "$0")" || exit 1
export PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_SYSROOT_DIR="$SYSROOT" PKG_CONFIG_LIBDIR="$SYSROOT/usr/lib/pkgconfig"
RUSTFLAGS="-L native=$SYSROOT/usr/lib" exec cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.41
