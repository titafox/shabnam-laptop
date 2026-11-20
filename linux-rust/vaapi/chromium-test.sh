#!/bin/sh
# chromium-test.sh — on-screen check that Chromium decodes video on the Rockchip hardware through VA-API
# (rockchip_drv_video.so). Runs INSIDE the Linux system as root; it opens one browser window for about a minute.
#   sh chromium-test.sh [zc|copy|sw] [nolog]
#     zc    hardware decode, frames handed to the GPU without a copy inside Chromium (default)
#     copy  hardware decode, Chromium's own copying path (no AcceleratedVideoDecodeLinuxZeroCopyGL)
#     sw    software decode, for comparison
#     nolog do not ask the driver for its per-call log (slightly less work per frame)
# It starts a SEPARATE Chromium (own throw-away profile) through /usr/local/bin/shabnam-browser as the user
# "shabnam", with the extra environment the wrapper's own start (env -i) does not pass yet, plays the 1080p30
# and 1080p60 H.264 clips from /var/tmp/pltest, and for each clip prints: frames shown and dropped (from the page
# title), CPU use, and which of its processes hold /dev/mpp_service. One screenshot: /tmp/shot-vaapi.png.
# At the end it closes its own window and processes only. Files it leaves: /var/tmp/vatest/out-<mode>/.
MODE=${1:-zc}; LOGGING=${2:-log}
T=/var/tmp/vatest; OUT=$T/out-$MODE; PAGE=$T/page/chromium-test.html
BROWSER_USER=shabnam
export PATH=/usr/local/bin:/usr/bin:/usr/sbin:/bin:/sbin XDG_RUNTIME_DIR=/run/user0
NIRI_SOCKET=$(ls -t /run/user0/niri.*.sock 2>/dev/null | head -1); export NIRI_SOCKET
[ -S "$NIRI_SOCKET" ] || { echo "no niri socket: the desktop is not running"; exit 1; }
[ -f "$PAGE" ] || { echo "missing $PAGE"; exit 1; }
[ -f /usr/lib/dri/rockchip_drv_video.so ] || echo "warning: /usr/lib/dri/rockchip_drv_video.so is not installed"

FEATURES=AcceleratedVideoDecoder,AcceleratedVideoDecodeLinuxGL,VaapiIgnoreDriverChecks
case $MODE in
  zc)   EXTRA="--enable-features=$FEATURES,AcceleratedVideoDecodeLinuxZeroCopyGL --ignore-gpu-blocklist" ;;
  copy) EXTRA="--enable-features=$FEATURES --ignore-gpu-blocklist" ;;
  sw)   EXTRA="--disable-accelerated-video-decode" ;;
  *) echo "usage: $0 [zc|copy|sw] [nolog]"; exit 1 ;;
esac

# The same hand-over of the compositor's socket that shabnam-browser does when root starts it.
share=/run/shabnam-share; socket=/run/user0/wayland-1
uid=$(id -u $BROWSER_USER) || exit 1
if [ ! -S $share/wayland-0 ]; then
  mkdir -p $share && chown root:$BROWSER_USER $share && chmod 750 $share &&
    chgrp $BROWSER_USER $socket && chmod 660 $socket && ln -f $socket $share/wayland-0 || { echo "cannot share $socket"; exit 1; }
fi
runtime=/run/user/$uid
mkdir -p $runtime && chown $BROWSER_USER:$BROWSER_USER $runtime && chmod 700 $runtime
rm -rf "$OUT"; mkdir -p "$OUT" && chown $BROWSER_USER:$BROWSER_USER "$OUT" && chmod 755 "$OUT"
PROFILE=$(setpriv --reuid $BROWSER_USER --regid $BROWSER_USER --init-groups mktemp -d /tmp/vaapi-test-profile.XXXXXX) || exit 1
[ "$LOGGING" = nolog ] && DRVLOG= || DRVLOG=$OUT/rk-vaapi.log

# The three variables after LD_LIBRARY_PATH (and RK_VAAPI_LOG, for this test only) are what the wrapper would
# have to pass through its "env -i" for everyday use.
cd /home/$BROWSER_USER || exit 1
umask 022
setpriv --reuid $BROWSER_USER --regid $BROWSER_USER --init-groups \
  env -i HOME=/home/$BROWSER_USER USER=$BROWSER_USER LOGNAME=$BROWSER_USER SHELL=/bin/sh \
    PATH=/usr/local/bin:/usr/bin:/bin LANG=zh_CN.UTF-8 XDG_RUNTIME_DIR=$runtime WAYLAND_DISPLAY=$share/wayland-0 \
    LD_LIBRARY_PATH=/opt/mali/mali:/opt/mali:/opt/mpp2/usr/lib/aarch64-linux-gnu \
    LIBVA_DRIVER_NAME=rockchip RKVA_HIDE_AV1=1 RKVA_HIDE_10BIT=1 ${DRVLOG:+RK_VAAPI_LOG=$DRVLOG} \
    SHABNAM_BROWSER_FLAGS="$EXTRA" \
    /usr/local/bin/shabnam-browser --user-data-dir="$PROFILE" --autoplay-policy=no-user-gesture-required \
      --enable-logging=stderr --vmodule='*/media/gpu/*=2,*vaapi*=2' "file://$PAGE" >"$OUT/chromium.log" 2>&1 &
MAIN=$!
echo "mode $MODE: browser pid $MAIN, profile $PROFILE"

HZ=$(getconf CLK_TCK)
NIRI=$(pidof niri-mali | awk '{print $1}')
# $MAIN and all processes below it: "pid ticks start-time" per line (ticks = user + system CPU time so far).
tree() {
  cat /proc/[0-9]*/stat 2>/dev/null | awk -v root="$MAIN" '
    { pid = $1; sub(/^.*\) /, ""); pp[pid] = $2; tk[pid] = $12 + $13; st[pid] = $20 }
    END { for (p in pp) { q = p; n = 0; while (q != root && q in pp && n++ < 50) q = pp[q]; if (q == root) print p, tk[p], st[p] } }'
}
ticks_tree() { tree | awk '{s += $2} END {print s + 0}'; }
ticks_of() { awk '{sub(/^.*\) /, ""); print $12 + $13}' /proc/$1/stat 2>/dev/null || echo 0; }
sys() { awk '/^cpu /{print $2+$3+$4+$5+$6+$7+$8+$9, $5+$6}' /proc/stat; }
title() { niri msg windows | awk -v pid="$MAIN" '/^Window ID/{t=""} /^  Title:/{t=$0} /^  PID:/{if ($2 == pid) print t}' | sed 's/^  Title: "//; s/"$//' | head -1; }
window_id() { niri msg windows | awk -v pid="$MAIN" '/^Window ID/{id=$3; sub(":", "", id)} /^  PID:/{if ($2 == pid) print id}' | head -1; }
mpp_holders() {
  tree | while read -r p _; do
    n=$(ls -l /proc/$p/fd 2>/dev/null | grep -c '/dev/mpp_service')
    [ "$n" -gt 0 ] && echo "    pid $p ($(tr '\0' '\n' < /proc/$p/cmdline | grep -m1 '^--type=' || echo browser)) holds /dev/mpp_service $n time(s)"
  done
}

# One line per second: clip, seconds playing, frames, dropped, CPU counters. Stops at "VAT done", an error, or 90 s.
: > "$OUT/samples"; shot=0; last=; held=
for n in $(seq 1 90); do
  sleep 1
  kill -0 $MAIN 2>/dev/null || { echo "the browser exited early; see $OUT/chromium.log"; break; }
  t=$(title); [ "$t" = "$last" ] || { echo "  title: $t"; last=$t; }
  case $t in
    "VAT done"*|"VAT error"*) break ;;
    "VAT p="*" t="*)
      p=$(echo "$t" | sed 's/.*p=\([0-9]*\).*/\1/'); s=$(echo "$t" | sed 's/.* t=\([0-9]*\).*/\1/')
      tot=$(echo "$t" | sed 's/.*tot=\([0-9]*\).*/\1/'); drop=$(echo "$t" | sed 's/.*drop=\([0-9]*\).*/\1/')
      set -- $(sys); echo "$p $s $tot $drop $(ticks_tree) $(ticks_of $NIRI) $1 $2 $(date +%s.%N)" >> "$OUT/samples"
      if [ "$s" -ge 8 ] && [ "$held" != "$p" ]; then
        held=$p; echo "  clip $p, /dev/mpp_service holders among this browser's processes:"; h=$(mpp_holders)
        [ -n "$h" ] && echo "$h" || echo "    none (the hardware decoder is NOT in use)"
      fi
      if [ "$p" = 60 ] && [ "$s" -ge 10 ] && [ $shot = 0 ]; then
        shot=1; niri msg action screenshot-screen --path /tmp/shot-vaapi.png >/dev/null 2>&1; sleep 1; chmod 644 /tmp/shot-vaapi.png 2>/dev/null
        echo "  screenshot: /tmp/shot-vaapi.png"
      fi ;;
  esac
done
FINAL=$(title)

echo "== result ($MODE) =="
echo "final title: $FINAL"
# Per clip, between the first sample at 4 s or later and the last one.
awk -v hz="$HZ" '
  $2 >= 4 { if (!($1 in a)) { a[$1] = $0 } b[$1] = $0 }
  END { for (p in a) { split(a[p], x); split(b[p], y); dt = y[9] - x[9]; if (dt <= 0) continue
      printf "clip 1080p%s: %.1f frames/s shown+dropped, %d dropped in %.0f s | browser %.0f%%  niri %.0f%% (of one core) | whole system busy %.0f%% of 4 cores\n",
        p, (y[3]-x[3])/dt, y[4]-x[4], dt, (y[5]-x[5])/hz/dt*100, (y[6]-x[6])/hz/dt*100, (1-(y[8]-x[8])/(y[7]-x[7]))*100 } }' "$OUT/samples"
if [ -n "$DRVLOG" ]; then
  if [ -s "$DRVLOG" ]; then
    echo "driver log: $(grep -c 'do_h264_decode: sending' "$DRVLOG") H.264 pictures sent to the hardware, $(grep -c 'ExportSurfaceHandle: surface=.*export_fd' "$DRVLOG") surface exports, $(grep -c -i -E 'fail|error' "$DRVLOG") lines with fail/error"
    grep -E 'ExportSurfaceHandle: surface=.*export_fd' "$DRVLOG" | head -1 | sed 's/^[^]]*] /  first export: /'
    grep -i -E 'fail|error' "$DRVLOG" | sed 's/^[^]]*] /  /' | sort | uniq -c | head -5
  else echo "driver log: empty — Chromium never loaded the VA-API driver"; fi
fi
echo "browser log lines about the decoder:"
grep -i -E 'vaapi|va_|VideoDecoder|decoder.*(fail|error)|libva' "$OUT/chromium.log" | cut -c1-220 | sort | uniq -c | sort -rn | head -12

# Close this instance only: its window first, then whatever of its process tree is left.
tree > "$OUT/pids"
id=$(window_id); [ -n "$id" ] && niri msg action close-window --id "$id" >/dev/null 2>&1
for n in 1 2 3 4 5; do kill -0 $MAIN 2>/dev/null || break; sleep 1; done
# A process is still "ours" only if the pid has the start time recorded above (a reused pid does not).
ours() { [ "$(awk '{sub(/^.*\) /, ""); print $20}' /proc/$1/stat 2>/dev/null)" = "$2" ]; }
while read -r p _ start; do ours $p $start && kill $p 2>/dev/null; done < "$OUT/pids"
sleep 1
while read -r p _ start; do ours $p $start && kill -9 $p 2>/dev/null; done < "$OUT/pids"
case $PROFILE in /tmp/vaapi-test-profile.*) rm -rf "$PROFILE" ;; esac
echo "closed; logs in $OUT"
