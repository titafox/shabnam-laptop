#!/bin/sh
# Cross-builds WordCraft (github.com/storytold/wordcraft, MIT OR Apache-2.0), a word processor
# (.docx), for the laptop.     Result: $WORK/out/wordcraft
#
# Like GridCraft it draws with wgpu only and needs ../wgpu-hal-30.0.1-mali-present.patch to start
# on the vendor Mali driver (see ../gridcraft/build.sh).
#
# What was read at this commit before building: the three build.rs files only read local files
# (contributor list, translations, optional fonts) or do Windows-only work; nothing is downloaded
# at build time apart from cargo's own crates.
# no-update-check.patch: the check at start and Help > Check for Updates are removed.
# noto-cjk-interface-font.patch: adds the place Void Linux keeps Noto Sans CJK to the list of fonts
# looked up for Chinese interface text (without it every Chinese label is drawn as boxes).
# Translations: CRAFT_I18N_DIR = storytold/craft-i18n at the commit in craft-i18n.rev (data only);
# the interface then follows LC_ALL / LC_MESSAGES / LANG (zh-hans and ru catalogs exist).
set -e
WORDCRAFT=079f478bd85df7bcc567e3bafa7d7219096f9c3c     # tag v0.18.1
CRAFT_I18N=1e515422afdda2f317af430c1f9d68a5f83935b4    # = craft-i18n.rev at that tag
here=$(cd "$(dirname "$0")" && pwd)
. "$here/../cross-env.sh"
checkout wordcraft https://github.com/storytold/wordcraft.git $WORDCRAFT
checkout craft-i18n-wordcraft https://github.com/storytold/craft-i18n.git $CRAFT_I18N
cd "$WORK/src/wordcraft"
[ "$(cat craft-i18n.rev)" = "$CRAFT_I18N" ]
git apply "$here/no-update-check.patch" "$here/noto-cjk-interface-font.patch"
export CRAFT_I18N_DIR="$WORK/src/craft-i18n-wordcraft" CRAFT_I18N_REQUIRED=1
export CARGO_TARGET_DIR="$WORK/target/wordcraft"
cargo fetch --locked --target $TRIPLE
hal=$(patched_crate wgpu-hal 30.0.1 "$here/../wgpu-hal-30.0.1-mali-present.patch")
cargo zigbuild --offline --release --target $TARGET -p wordcraft --config "patch.crates-io.wgpu-hal.path='$hal'"
git diff --stat -- Cargo.lock     # only the wgpu-hal entry may differ
cp "$CARGO_TARGET_DIR/$TRIPLE/release/wordcraft" "$WORK/out/wordcraft"
