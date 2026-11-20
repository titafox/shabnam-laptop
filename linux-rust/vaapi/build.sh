#!/bin/sh
# Cross-builds the VA-API driver for the laptop's Rockchip video decoder (rockchip_drv_video.so) on the Mac
# for aarch64, glibc 2.41, with zig. Never build on the device. Needs: zig, git.
#   SYSROOT = copy of the device's /usr tree holding the libva headers and library (Void package libva-devel:
#             usr/include/va, usr/lib/libva.so.2 with a libva.so link) and the MPP headers and library
#             (usr/include/rockchip, usr/lib/librockchip_mpp.so) from the MPP 1.5.0 the device runs (/opt/mpp2).
#   WORK    = a scratch directory outside the repository; the source is cloned into $WORK/src.
# Result: $WORK/rockchip_drv_video.so -> install as /usr/local/lib/shabnam-va/rockchip_drv_video.so on the device
# (root, 755). Not in libva's own directory (/usr/lib/dri): there every program would load it by itself; from
# the private directory only shabnam-browser does (it sets LIBVA_DRIVERS_PATH; SHABNAM_HW_VIDEO=0 turns it off).
# Source: github.com/defcom5-rockchip/rockchip-vaapi, tag v4.0.0 (LGPL-2.1), plus the *.patch files next to
# this script, if any. Built without RGA (the Makefile's RGA=no): 8-bit frames are exported as they are, only
# 10-bit frames would be converted on the CPU.
: "${SYSROOT:?set SYSROOT}" "${WORK:?set WORK}"
set -e
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$WORK" && cd "$WORK"
[ -d src ] || { git clone -q -b v4.0.0 --depth 1 https://github.com/defcom5-rockchip/rockchip-vaapi.git src
                for p in "$here"/*.patch; do [ -f "$p" ] && git -C src apply "$p"; done; }
cd src
# The driver finds the MPP library by itself (rpath), whatever environment the program that loads it runs in.
zig cc -target aarch64-linux-gnu.2.41 -O2 -Wall -Wextra -Wno-unused-function -Wno-unused-parameter -fPIC -shared \
  -DGST_USE_UNSTABLE_API -Isrc -Isrc/av1 -I"$SYSROOT/usr/include" -I"$SYSROOT/usr/include/rockchip" \
  src/rockchip_drv_video.c src/h264.c src/hevc.c src/av1.c src/av1/gstav1bitwriter.c src/av1/gstcompat.c \
  -L"$SYSROOT/usr/lib" -lva -lrockchip_mpp -lpthread -ldl -Wl,-z,defs \
  -Wl,-rpath,/opt/mpp2/usr/lib/aarch64-linux-gnu -o "$WORK/rockchip_drv_video.so"
