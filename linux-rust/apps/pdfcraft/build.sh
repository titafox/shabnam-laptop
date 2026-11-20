#!/bin/sh
# Cross-builds PdfCraft (github.com/storytold/pdfcraft, MIT OR Apache-2.0) for the laptop.
# Result: $WORK/out/pdfcraft     Install: ../install-app.sh pdfcraft
#
# What was read at this commit before building: every build.rs in the tree only reads local files
# (fonts, translations, contributor list) or does Windows-only work; nothing is downloaded at build
# time apart from cargo's own crates (among them storytold's egui and winit forks and craft-libs,
# all pinned by git revision in Cargo.toml / Cargo.lock; --locked keeps them there).
#
# no-update-check.patch: the app asks GitHub for a newer release at start and can install one
# (craft-update). The patch removes both checks (at start and Help > Check for updates), so the
# updater never learns of a release and never downloads anything.
#
# noto-cjk-interface-font.patch: for Chinese interface text the program looks for a few installed
# fonts by file name (WenQuanYi, Droid, AR PL) and Noto Sans CJK is not among them, so every Chinese
# label was drawn as boxes. The patch adds /usr/share/fonts/noto/NotoSansCJK-Regular.ttc.
#
# Translations: CRAFT_I18N_DIR is a checkout of storytold/craft-i18n at the commit pdfcraft names
# in craft-i18n.rev (data files only: .tsv catalogs). Without it the interface is English only;
# with it, it follows LC_ALL / LC_MESSAGES / LANG (zh_CN -> zh-hans, ru_RU -> ru).
set -e
PDFCRAFT=b503b2e62dfeec5598f344e4e287d9c9cbdb34fa      # tag v0.21.1
CRAFT_I18N=575cfdef63227b14daf173b8cc87928bfdfa325e    # = craft-i18n.rev at that tag
here=$(cd "$(dirname "$0")" && pwd)
. "$here/../cross-env.sh"
checkout pdfcraft https://github.com/storytold/pdfcraft.git $PDFCRAFT
checkout craft-i18n https://github.com/storytold/craft-i18n.git $CRAFT_I18N
cd "$WORK/src/pdfcraft"
[ "$(cat craft-i18n.rev)" = "$CRAFT_I18N" ]
git apply "$here/no-update-check.patch" "$here/noto-cjk-interface-font.patch"
export CRAFT_I18N_DIR="$WORK/src/craft-i18n" CRAFT_I18N_REQUIRED=1
export CARGO_TARGET_DIR="$WORK/target/pdfcraft"
cargo zigbuild --locked --release --target $TARGET -p pdfcraft
cp "$CARGO_TARGET_DIR/$TRIPLE/release/pdfcraft" "$WORK/out/pdfcraft"
