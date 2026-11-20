#!/bin/sh
# Cross-builds GridCraft (github.com/storytold/gridcraft, MIT OR Apache-2.0), a spreadsheet, for the laptop.
# Result: $WORK/out/gridcraft
#
# GridCraft draws with wgpu only (no glow renderer); on Linux it asks wgpu for its OpenGL backend
# (GLES through EGL) unless WGPU_BACKEND says otherwise.
#
# What was read at this commit before building: the two build.rs files only read local files
# (contributor list, optional fonts) or do Windows-only work; nothing is downloaded at build time
# apart from cargo's own crates (storytold's egui/winit forks and craft-libs, pinned by revision).
# Translations are compiled in (crates/ui-egui/src/i18n/: zh, ru and 15 more; English is the source)
# but the language is taken from the system locale only at the first start and then from ui.json;
# language-follows-locale.patch takes it from LC_ALL / LC_MESSAGES / LANG at every start.
#
# ../wgpu-hal-30.0.1-mali-present.patch: one line in wgpu's EGL code. On Linux wgpu accepts a window
# only when EGL offers a config with EGL_NATIVE_RENDERABLE; the vendor Mali driver has none, and
# without the patch GridCraft ends at start with "No suitable graphics adapter found ... gl not
# compatible with provided surface". The patch uses the rule wgpu already applies on Android.
# The crate is copied from cargo's cache and patched (patched_crate in ../cross-env.sh).
#
# no-update-check.patch: same purpose as pdfcraft's — the check at start and Help > Check for
# Updates are removed, so the updater (craft-update) never learns of a release and downloads nothing.
set -e
GRIDCRAFT=476448d7d9f3a5f8e93fcf7d98b47fd52293fe7f     # tag v0.19.1
here=$(cd "$(dirname "$0")" && pwd)
. "$here/../cross-env.sh"
checkout gridcraft https://github.com/storytold/gridcraft.git $GRIDCRAFT
cd "$WORK/src/gridcraft"
git apply "$here/no-update-check.patch" "$here/language-follows-locale.patch"
export CARGO_TARGET_DIR="$WORK/target/gridcraft"
cargo fetch --locked --target $TRIPLE
hal=$(patched_crate wgpu-hal 30.0.1 "$here/../wgpu-hal-30.0.1-mali-present.patch")
cargo zigbuild --offline --release --target $TARGET -p gridcraft --config "patch.crates-io.wgpu-hal.path='$hal'"
git diff --stat -- Cargo.lock     # only the wgpu-hal entry may differ
cp "$CARGO_TARGET_DIR/$TRIPLE/release/gridcraft" "$WORK/out/gridcraft"
