#!/bin/sh
# Cross-builds Ferrite (github.com/OlaProeis/Ferrite, MIT), a text / Markdown editor, for the laptop.
# Result: $WORK/out/ferrite      Needs SYSROOT (fontconfig, freetype, zlib and their pkg-config files).
#
# Pinned to v0.3.0, not the newer v0.3.1: v0.3.1 adds a hard dependency on wry (WebKitGTK) and GTK 3
# for video embeds, which would pull a second browser engine onto the laptop.
#
# What was read at this commit before building: build.rs does Windows-only work; nothing is
# downloaded at build time apart from cargo's crates. The program does not look for updates by
# itself: the only request to GitHub is behind the "Check for updates" button in Settings > About,
# and it only reports the version (nothing is downloaded or installed).
#
# Features: without "high-perf-alloc" (jemalloc, a C library with its own configure script).
# RUSTUP_TOOLCHAIN=stable: rust-toolchain.toml asks for exactly 1.92.0; any later stable builds it.
#
# language-follows-locale.patch: Ferrite takes its language from the system locale only at the very
# first start and afterwards from its settings file. The patch takes it from LC_ALL / LC_MESSAGES /
# LANG at every start. Languages it really has: English and Simplified Chinese (also German,
# Spanish, Japanese). locales/ru.yaml exists but is a stub (13 of about 740 strings filled in), so
# a Russian session gets the English interface.
set -e
FERRITE=ac4f36695e5b60a0a41fe64a458e0c2279fca13c       # tag v0.3.0
here=$(cd "$(dirname "$0")" && pwd)
: "${SYSROOT:?set SYSROOT to the directory holding the device sysroot (it contains usr/lib)}"
. "$here/../cross-env.sh"
checkout ferrite https://github.com/OlaProeis/Ferrite.git $FERRITE
cd "$WORK/src/ferrite"
git apply "$here/language-follows-locale.patch"
export CARGO_TARGET_DIR="$WORK/target/ferrite" RUSTUP_TOOLCHAIN=stable
RUSTFLAGS="-L native=$SYSROOT/usr/lib" \
  cargo zigbuild --locked --release --target $TARGET --no-default-features --features bundle-icon
cp "$CARGO_TARGET_DIR/$TRIPLE/release/ferrite" "$WORK/out/ferrite"
