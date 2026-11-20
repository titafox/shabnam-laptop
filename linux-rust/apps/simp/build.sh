#!/bin/sh
# Cross-builds Simp (github.com/Kl4rry/simp, Apache-2.0), an image viewer with light editing
# (rotate, crop, resize, save as), for the laptop.      Result: $WORK/out/simp
#
# Pinned to a commit on main after v3.11.0, not to the tag: at the tag wgpu is built with the
# Metal and DX12 backends only; the commit below adds "gles" (and wgpu 30.0.1, the version
# ../wgpu-hal-30.0.1-mali-present.patch is made for; see ../gridcraft/build.sh for why it is needed).
# sha256 of github.com/Kl4rry/simp/archive/<commit>.tar.gz when it was read:
#   541b824945c646307ccc88493b0d23e610c456decea4d8b818518a79804339df
#
# What was read at this commit before building: all of src/ (7,500 lines) and build.rs. The program
# has no update check and makes no network connection; Help > Repository / Report Bug hand a
# github.com address to the system's URL opener. build.rs asks git for the commit hash and runs
# `cargo about` — no-cargo-about.patch replaces the latter with a fixed one-line licence page.
#
# adapter-limits.patch: Simp asks wgpu for its desktop default limits; the laptop's GLES driver has
# less (4 colour attachments, not 8) and the unpatched program ends at start with
# "LimitsExceeded ... max_color_attachments". The patch asks for the limits the adapter reports.
#
# Formats in this build (default features): JPEG (EXIF rotation applied), PNG/APNG, WebP, GIF, BMP,
# TIFF, SVG (resvg), ICO, TGA, QOI, PSD, OpenEXR, camera raw. Not built: HEIC ("heif": libheif, C++),
# AVIF ("avif": dav1d, C), JPEG XL ("jxl": libjxl, C++). The only C code compiled in is libwebp
# (crate libwebp-sys2, feature "static"), built by cc with zig.
#
# idle-redraw.patch: upstream draws a new frame every time its event loop wakes up, and on Wayland
# showing a frame wakes it, so a still picture was redrawn at the screen's refresh rate for ever
# (18.6 % of one core of the laptop). The patch draws when something asks for it: input, a
# resize, an animation's next frame, egui's own repaint requests, a message from a worker thread.
#
# zh-ru-interface.patch: upstream is English only. The patch adds src/i18n.rs — a table of the
# interface's 118 texts in Simplified Chinese and Russian, looked up by the English text
# (`tr("Open")`), language from LC_ALL / LC_MESSAGES / LANG, English for anything else — and puts
# `tr(..)` around those texts where they are shown. It also gives egui the Simplified Chinese face
# of Noto Sans CJK (/usr/share/fonts/noto/NotoSansCJK-Regular.ttc, face 2) as the last font, in
# every language, so that Chinese text and file names are drawn (egui's own fonts have Latin and
# Cyrillic only). Left in English: format and pixel-type names (PNG, Rgba8, Lanczos ...), key
# names in shortcuts, and the three error texts that carry a decoder's own message.
set -e
SIMP=b6e5f05c2f5f53f91fdf587f90dacf53e0141947          # main, 2026-10-05 (v3.11.0 + 7 commits)
here=$(cd "$(dirname "$0")" && pwd)
. "$here/../cross-env.sh"
checkout simp https://github.com/Kl4rry/simp.git $SIMP
cd "$WORK/src/simp"
git apply "$here/no-cargo-about.patch" "$here/adapter-limits.patch" "$here/idle-redraw.patch" \
          "$here/zh-ru-interface.patch"
export CARGO_TARGET_DIR="$WORK/target/simp"
cargo fetch --locked --target $TRIPLE
hal=$(patched_crate wgpu-hal 30.0.1 "$here/../wgpu-hal-30.0.1-mali-present.patch")
cargo zigbuild --offline --release --target $TARGET --config "patch.crates-io.wgpu-hal.path='$hal'"
git diff --stat -- Cargo.lock     # only the wgpu-hal entry may differ
cp "$CARGO_TARGET_DIR/$TRIPLE/release/simp" "$WORK/out/simp"
