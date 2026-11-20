#!/bin/sh
# install.sh NAME BINARY [ICON.png] — puts one cross-built program on the laptop (over adb):
#   BINARY                         -> /usr/local/lib/NAME/NAME        (root:root 755)
#   ICON.png (taken from the program's own source tree when its licence allows; never stored in
#             this repository)     -> /usr/local/share/shabnam-apps/NAME.png
#   ../device/shabnam-userapp and every ../device/shabnam-* launcher named in LAUNCHERS -> /usr/local/bin
#   ../device/app-names/DESKTOP    -> /usr/local/share/applications
# Files are written under a temporary name and renamed into place; umask 022, nothing world-writable.
#   LAUNCHERS="shabnam-pdf" DESKTOP="shabnam-pdf.desktop" ./install.sh pdfcraft $WORK/out/pdfcraft icon.png
set -e
name=$1 bin=$2 icon=$3
: "${name:?usage: install.sh NAME BINARY [ICON.png]}" "${bin:?}"
ADB="adb -s ${SHABNAM_ADB:-192.168.1.33:5555}"
here=$(cd "$(dirname "$0")" && pwd)
stage=/data/local/tmp/shabnam-app-$name
$ADB shell "rm -rf $stage; mkdir -p $stage"
$ADB push "$bin" $stage/bin >/dev/null
[ -z "$icon" ] || $ADB push "$icon" $stage/icon.png >/dev/null
for f in shabnam-userapp $LAUNCHERS; do $ADB push "$here/../device/$f" $stage/l-$f >/dev/null; done
for f in $DESKTOP; do $ADB push "$here/../device/app-names/$f" $stage/d-$f >/dev/null; done
$ADB shell "su 0 sh -c '
  umask 022; set -e; r=/data/void; s=$stage
  put() { cp \$1 \$2.new; chown 0:0 \$2.new; chmod \$3 \$2.new; mv -f \$2.new \$2; }
  mkdir -p \$r/usr/local/lib/$name \$r/usr/local/share/shabnam-apps \$r/usr/local/share/applications
  chmod 755 \$r/usr/local/lib/$name \$r/usr/local/share/shabnam-apps
  put \$s/bin \$r/usr/local/lib/$name/$name 755
  [ ! -f \$s/icon.png ] || put \$s/icon.png \$r/usr/local/share/shabnam-apps/$name.png 644
  for f in \$s/l-*; do put \$f \$r/usr/local/bin/\${f##*/l-} 755; done
  for f in \$s/d-*; do [ -f \$f ] && put \$f \$r/usr/local/share/applications/\${f##*/d-} 644; done
  rm -rf \$s
  ls -la \$r/usr/local/lib/$name
'"
