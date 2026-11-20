#!/bin/sh
# Cross-builds shabnam-video on the Mac for the laptop (aarch64, glibc 2.41) with zig. Never build on the device.
#   SYSROOT = a copy of the device's /usr tree holding the GStreamer, GLib and Wayland client headers,
#             pkg-config files and libraries (gstreamer1-devel, gst-plugins-base1-devel, wayland-devel).
#   OUT     = where the binary goes
# protocol/ holds wayland-scanner output (xdg-shell, viewporter, linux-dmabuf-v1, presentation-time).
: "${SYSROOT:?set SYSROOT to the directory holding the device sysroot (it contains usr/lib)}"
: "${OUT:?set OUT to the output file}"
cd "$(dirname "$0")" || exit 1
export PKG_CONFIG_SYSROOT_DIR="$SYSROOT" PKG_CONFIG_LIBDIR="$SYSROOT/usr/lib/pkgconfig:$SYSROOT/usr/share/pkgconfig"
PKGS="gstreamer-1.0 gstreamer-app-1.0 gstreamer-video-1.0 gstreamer-allocators-1.0 glib-2.0 gobject-2.0"
# shellcheck disable=SC2046
exec zig cc -target aarch64-linux-gnu.2.41 -O2 -Wall -Wno-unused-parameter -Iprotocol \
  $(pkg-config --cflags $PKGS) -I"$SYSROOT/usr/include" \
  main.c protocol/xdg-shell.c protocol/viewporter.c protocol/linux-dmabuf-v1.c protocol/presentation-time.c \
  -L"$SYSROOT/usr/lib" $(pkg-config --libs-only-l $PKGS) -lwayland-client -lm -o "$OUT"
