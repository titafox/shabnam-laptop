#!/bin/sh
# Cross-builds Gem Player (github.com/JamesMoreau/gem-player, MIT), a music player with a library,
# playlists (.m3u), a queue, cover art and tags, for the laptop.     Result: $WORK/out/gem-player
# Needs SYSROOT (libasound, libdbus-1, fontconfig, freetype; all already on the laptop).
#
# Pinned to tag v1.2.1. sha256 of github.com/JamesMoreau/gem-player/archive/<commit>.tar.gz when it
# was read: 6b67579930ccfff4f687b797d1b49fabce510ac69b51e93dcd2b8d52a2120ee0
# Upstream keeps no Cargo.lock in its repository; ./Cargo.lock (made on 2026-11-20 with
# `cargo generate-lockfile` after the patches) pins every crate by version and checksum.
#
# What was read at this commit before building: all of src/ (4,700 lines) and build.rs (Windows
# only). The program has no update check, no telemetry and opens no network connection; the links
# in Settings > About are handed to the system's URL opener when clicked. It writes its settings,
# the playing track's cover (one PNG) and .m3u playlists (inside the library folder); it does not
# change audio files.
#
# builds-on-linux.patch: upstream publishes macOS and Windows builds only and its Linux code stops
# at the native menu bar (crate muda, GTK 3), which does not compile. The patch leaves the menu bar
# out on Linux; nothing else is touched.
#
# shabnam.patch:
#   - sound: cpal's "pulseaudio" feature (a pure-Rust PulseAudio-protocol client; cpal uses it
#     instead of ALSA when PULSE_SERVER / XDG_RUNTIME_DIR lead to a server). ../../device/shabnam-music
#     sets PULSE_SERVER to the link the unprivileged user may use.
#   - drawing: eframe's glow renderer (OpenGL ES) instead of wgpu.
#   - `gem-player FILE...` plays the files (one file: followed by the rest of its folder).
#   - the library is ~/Music until another folder is stored (the folder chooser needs the desktop
#     portal service, which this system does not run).
#   - the interface language follows LC_ALL / LC_MESSAGES / LANG at every start.
#   - the library folder's watcher ignores "a file was opened / read" events. On Linux they are
#     events too, so each scan of the library (which reads every file's tags) started the next
#     one, for ever (365 scans in 95 seconds on the laptop before the patch).
#   - redraws: 15 a second while a track plays and about 2 a second otherwise, instead of 30 at
#     all times (53 % of one core with nothing playing on the laptop before the patch).
#   - the Wayland app id is "gem-player" (upstream: the window title "Gem Player").
#   - Chinese text uses the Simplified Chinese face of Noto Sans CJK, not the collection's first
#     (Japanese) face; files without tags are listed by file name instead of being skipped.
# locales/ru.TOML is renamed to ru.toml: the translation loader matches lower-case extensions
# only, so on a case-sensitive file system the Russian catalog was not compiled in.
#
# Languages really present (locales/*.toml, 92 strings each): English, Simplified Chinese, Russian
# (also German, Spanish, French, Japanese).
# Formats (rodio "symphonia-all" = symphonia 0.5): MP3, FLAC, M4A/AAC/ALAC, Ogg Vorbis, WAV, AIFF,
# and Opus through rodio-0.23.0-opus.patch.
#
# rodio-0.23.0-opus.patch (applied to the rodio crate, not to Gem Player): symphonia 0.5 reads Ogg
# Opus files but has no Opus decoder, and rodio offers no way to add a codec, so .opus files were
# listed and failed to play. The patch adds src/decoder/opus.rs — symphonia's Decoder interface
# over the crate unsafe-libopus 0.2.0 — and makes rodio's symphonia backend use a codec list with
# it. unsafe-libopus is the reference decoder (libopus 1.3.1) translated to Rust by c2rust: cargo
# builds it like any crate (no C compiler, no system library, nothing prebuilt). Chosen over the
# crate opus-decoder 0.1.1 (safe Rust, written anew), which was tried first: on a 16 kbit/s mono
# speech file (hybrid mode) 30 of its 501 packets came out different from libopus (up to 8 % of
# full scale); unsafe-libopus matched ffmpeg's libopus 1.6.1 within 16-bit rounding (81 dB or
# better) on seven test encodings (CELT, SILK narrow/wide band, hybrid; mono, stereo; 2.5-60 ms).
# Mono and stereo only. The crate is taken from crates.io by name, version and the sha256 that
# Cargo.lock held for it before the patch.
#
# open-in-running-player.patch: `gem-player FILE...` while a player runs hands the files to that
# player through $XDG_RUNTIME_DIR/gem-player.sock and ends; the running player plays them (file,
# then the rest of its folder, as at start). Upstream has no single-instance mechanism.
set -e
GEM=ab58f321a4c8cd035bd86054c641510361580797          # tag v1.2.1
here=$(cd "$(dirname "$0")" && pwd)
: "${SYSROOT:?set SYSROOT to the directory holding the device sysroot (it contains usr/lib)}"
. "$here/../cross-env.sh"
checkout gem-player https://github.com/JamesMoreau/gem-player.git $GEM
cd "$WORK/src/gem-player"
git apply "$here/builds-on-linux.patch" "$here/shabnam.patch" "$here/open-in-running-player.patch"
mv locales/ru.TOML locales/ru.toml
cp "$here/Cargo.lock" Cargo.lock
# The sysroot has libasound.so but not alsa-lib's development files; alsa-sys only needs to be
# told where the library is.
mkdir -p "$WORK/pc"
printf 'Name: alsa\nDescription: ALSA library\nVersion: 1.2.14\nLibs: -L/usr/lib -lasound\nCflags:\n' > "$WORK/pc/alsa.pc"
export PKG_CONFIG_LIBDIR="$PKG_CONFIG_LIBDIR:$WORK/pc"
export CARGO_TARGET_DIR="$WORK/target/gem-player" RUSTUP_TOOLCHAIN=stable
# rodio 0.23.0 with Opus: the published crate, checked against its sha256, plus our patch. Cargo
# is pointed at the directory; Cargo.lock names rodio without a registry source for that reason.
rodio=$WORK/src/rodio-0.23.0-shabnam
rm -rf "$rodio" "$rodio.crate" && mkdir -p "$rodio"
curl -sSfL -o "$rodio.crate" https://static.crates.io/crates/rodio/rodio-0.23.0.crate
echo "36e3ee0a5ca418bac59a15d5506e498ab138359edbb67257a6b8a3b93d411f39  $rodio.crate" | shasum -a 256 -c - >/dev/null
tar -xzf "$rodio.crate" -C "$rodio" --strip-components 1
patch -s -p1 -d "$rodio" < "$here/rodio-0.23.0-opus.patch"
RUSTFLAGS="-L native=$SYSROOT/usr/lib" \
  cargo zigbuild --locked --release --target $TARGET --config "patch.crates-io.rodio.path='$rodio'"
cp "$CARGO_TARGET_DIR/$TRIPLE/release/gem-player" "$WORK/out/gem-player"
