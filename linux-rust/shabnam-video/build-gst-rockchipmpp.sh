#!/bin/sh
# Cross-builds Rockchip's GStreamer plugin (libgstrockchipmpp.so: mppvideodec and friends) on the Mac for the
# laptop (aarch64, glibc 2.41). Never build on the device. Needs: zig, meson, ninja, pkg-config, patchelf, git.
#   SYSROOT = copy of the device's /usr tree with the GStreamer and GLib headers, pkg-config files and
#             libraries (gstreamer1-devel, gst-plugins-base1-devel, libdrm-devel) and, copied into it, the MPP
#             headers (usr/include/rockchip), librockchip_mpp.so and rockchip_mpp.pc (libdir=/usr/lib) from the
#             MPP 1.5.0 package the device runs (/opt/mpp2). usr/lib64 must be a link to usr/lib.
#             Also librga (github.com/airockchip/librga, commit 2b32edc, api 1.10.6): include/*.h copied to
#             usr/include/rga/, libs/Linux/gcc-aarch64/librga.so to usr/lib/, and a librga.pc next to the
#             others. The same librga.so is installed on the device as /usr/local/lib/librga.so.
#   WORK    = an empty scratch directory outside the repository.
# Result: $WORK/libgstrockchipmpp.so -> install as /usr/lib/gstreamer-1.0/libgstrockchipmpp.so on the device.
# Source: github.com/JeffyCN/mirrors, branch gstreamer-rockchip, commit bff0659 (2026-10-22), plus
# ../patches/gstreamer-rockchip-mpp-1.5.0.patch (the JPEG decoder used an MPP call newer than 1.5.0).
# RGA (the 2D accelerator) is compiled in: the player asks the decoder for BGRx frames, because the
# compositor cannot show NV12 dmabufs on the Mali driver. The GPU converter, the X11 sink and kmssrc are left out.
: "${SYSROOT:?set SYSROOT}" "${WORK:?set WORK}"
set -e
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$WORK/tools" && cd "$WORK"
[ -d src ] || { git clone -b gstreamer-rockchip https://github.com/JeffyCN/mirrors.git src
                git -C src checkout bff0659 && git -C src apply "$here/../patches/gstreamer-rockchip-mpp-1.5.0.patch"; }
printf '#!/bin/sh\nexec zig cc -target aarch64-linux-gnu.2.41 "$@"\n' > tools/zcc
printf '#!/bin/sh\nexec zig ar "$@"\n' > tools/zar
chmod +x tools/zcc tools/zar
cat > cross.ini <<INI
[binaries]
c = '$WORK/tools/zcc'
ar = '$WORK/tools/zar'
strip = 'true'
pkg-config = 'pkg-config'
[host_machine]
system = 'linux'
cpu_family = 'aarch64'
cpu = 'aarch64'
endian = 'little'
[properties]
sys_root = '$SYSROOT'
pkg_config_libdir = '$SYSROOT/usr/lib/pkgconfig:$SYSROOT/usr/share/pkgconfig'
INI
rm -rf build
meson setup build src --cross-file cross.ini --buildtype=release \
  -Drkximage=disabled -Dkmssrc=disabled -Drga=enabled -Dgpu=disabled -Drockchipmpp=enabled
ninja -C build
cp build/gst/rockchipmpp/libgstrockchipmpp.so "$WORK/libgstrockchipmpp.so"
# The plugin finds the MPP and RGA libraries by itself, whatever environment GStreamer's plugin scanner runs in.
patchelf --set-rpath /opt/mpp2/usr/lib/aarch64-linux-gnu:/usr/local/lib "$WORK/libgstrockchipmpp.so"
