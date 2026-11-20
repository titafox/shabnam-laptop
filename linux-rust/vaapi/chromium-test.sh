#!/bin/sh
# chromium-test.sh — on-screen check that Chromium decodes video on the Rockchip hardware through VA-API
# (rockchip_drv_video.so). Runs INSIDE the Linux system as root; it opens one browser window for about a minute.
#   sh chromium-test.sh [zc|copy|plain|sw] [nolog] [clips, default 30,60 — add 9 for the VP9 clip: 30,60,9]
#     plain hardware decode the way shabnam-browser starts it, no extra Chromium flags
#     zc    the same plus the flags other guides give (AcceleratedVideoDecodeLinuxZeroCopyGL, --ignore-gpu-blocklist);
#           measured: no difference here (default)
#     copy  as zc without AcceleratedVideoDecodeLinuxZeroCopyGL; measured: no difference either
#     sw    software decode (SHABNAM_HW_VIDEO=0), for comparison
#     nolog do not ask the driver for its per-call log (slightly less work per frame)
# It starts a SEPARATE Chromium (own throw-away profile) through /usr/local/bin/shabnam-browser as the user
# "shabnam", plays the clips from /var/tmp/vatest/clips (30 = H.264 1080p30, 60 = H.264 1080p60, 9 = VP9 1080p60;
# copies of the test clips must be put there first), and for each clip prints: frames shown and dropped (from
# the page title), CPU use, and which of its processes hold /dev/mpp_service.
# At the end it closes its own window and processes only. It leaves /var/tmp/vatest/out-<mode>/ (root only):
# one screenshot per clip (shot-<clip>.png), the browser's output and a copy of the driver's log.
umask 022
MODE=${1:-zc}; LOGGING=${2:-log}; CLIPS=${3:-30,60}
VA_DIR=${VA_DIR:-/usr/local/lib/shabnam-va}   # where rockchip_drv_video.so lives (not libva's default directory)
T=/var/tmp/vatest; OUT=$T/out-$MODE; PAGE=$T/page/chromium-test.html
BROWSER_USER=shabnam
export PATH=/usr/local/bin:/usr/bin:/usr/sbin:/bin:/sbin XDG_RUNTIME_DIR=/run/user0
NIRI_SOCKET=$(ls -t /run/user0/niri.*.sock 2>/dev/null | head -1); export NIRI_SOCKET
[ -S "$NIRI_SOCKET" ] || { echo "no niri socket: the desktop is not running"; exit 1; }
[ -f "$PAGE" ] || { echo "missing $PAGE"; exit 1; }
for f in h264_1080p30.mp4 h264_1080p60.mp4 vp9_1080p60.webm; do
  setpriv --reuid $BROWSER_USER --regid $BROWSER_USER --init-groups test -r $T/clips/$f || { echo "FAILED: clip $T/clips/$f is missing or not readable by $BROWSER_USER"; exit 1; }
done
[ "$MODE" = sw ] || [ -r $VA_DIR/rockchip_drv_video.so ] || { echo "FAILED: $VA_DIR/rockchip_drv_video.so is not installed"; exit 1; }

HW=1
FEATURES=AcceleratedVideoDecoder,AcceleratedVideoDecodeLinuxGL,VaapiIgnoreDriverChecks
case $MODE in
  zc)   EXTRA="--enable-features=$FEATURES,AcceleratedVideoDecodeLinuxZeroCopyGL --ignore-gpu-blocklist" ;;
  copy) EXTRA="--enable-features=$FEATURES --ignore-gpu-blocklist" ;;
  sw)   EXTRA=; HW=0 ;;
  plain) EXTRA= ;;    # the driver is offered (environment only), no extra Chromium flags
  *) echo "usage: $0 [zc|copy|plain|sw] [nolog] [clips]"; exit 1 ;;
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
# Two places, never mixed: $OUT belongs to root (mode 700, under a root-owned directory) and holds everything this
# script writes and reads back (samples, process list, browser output, screenshots). $PROFILE belongs to the
# browser user and holds what the browser writes itself (its profile, the driver's log); this script only ever
# reads from there as that user, and never acts on what it finds.
AS_USER="setpriv --reuid $BROWSER_USER --regid $BROWSER_USER --init-groups"
[ "$(stat -c %u:%a $T)" = 0:755 ] || { echo "FAILED: $T must belong to root, mode 755"; exit 1; }
rm -rf "$OUT"; mkdir -m 700 "$OUT" || exit 1
PROFILE=$($AS_USER mktemp -d /tmp/vaapi-test-profile.XXXXXX) || exit 1
[ "$LOGGING" = nolog ] && DRVLOG= || DRVLOG=$PROFILE/rk-vaapi.log

# shabnam-browser itself points Chromium at the driver (LIBVA_DRIVERS_PATH and friends) unless SHABNAM_HW_VIDEO=0;
# RK_VAAPI_LOG, for this test only, makes the driver write its per-call log.
cd /home/$BROWSER_USER || exit 1
umask 022
setpriv --reuid $BROWSER_USER --regid $BROWSER_USER --init-groups \
  env -i HOME=/home/$BROWSER_USER USER=$BROWSER_USER LOGNAME=$BROWSER_USER SHELL=/bin/sh \
    PATH=/usr/local/bin:/usr/bin:/bin LANG=zh_CN.UTF-8 XDG_RUNTIME_DIR=$runtime WAYLAND_DISPLAY=$share/wayland-0 \
    LD_LIBRARY_PATH=/opt/mali/mali:/opt/mali:/opt/mpp2/usr/lib/aarch64-linux-gnu \
    SHABNAM_HW_VIDEO=$HW ${DRVLOG:+RK_VAAPI_LOG=$DRVLOG} \
    SHABNAM_BROWSER_FLAGS="$EXTRA" \
    /usr/local/bin/shabnam-browser --user-data-dir="$PROFILE" --autoplay-policy=no-user-gesture-required \
      --enable-logging=stderr  --vmodule='*/media/gpu/*=2,*vaapi*=2,*video_decoder*=2,*decoder_selector*=2' "file://$PAGE?clips=$CLIPS" >"$OUT/chromium.log" 2>&1 &
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
    [ "$n" -gt 0 ] && echo "    pid $p ($(tr '\0' ' ' < /proc/$p/cmdline | grep -o -m1 -e '--type=[a-z-]*' | head -1 | grep . || echo main browser process)) holds /dev/mpp_service $n time(s)"
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
      p=$(echo "$t" | sed 's/^VAT p=\([0-9]*\).*/\1/'); s=$(echo "$t" | sed 's/.* t=\([0-9]*\).*/\1/')
      tot=$(echo "$t" | sed 's/.*tot=\([0-9]*\).*/\1/'); drop=$(echo "$t" | sed 's/.*drop=\([0-9]*\).*/\1/')
      set -- $(sys); echo "$p $s $tot $drop $(ticks_tree) $(ticks_of $NIRI) $1 $2 $(date +%s.%N)" >> "$OUT/samples"
      if [ "$s" -ge 8 ] && [ "$held" != "$p" ]; then
        held=$p; echo "  clip $p, /dev/mpp_service holders among this browser's processes:"; h=$(mpp_holders)
        [ -n "$h" ] && echo "$h" || echo "    none (the hardware decoder is NOT in use)"
      fi
      if [ "$s" -ge 10 ] && [ "$shot" != "$p" ]; then
        shot=$p; niri msg action screenshot-screen --path $OUT/shot-$p.png >/dev/null 2>&1; sleep 1
        echo "  screenshot: $OUT/shot-$p.png"
      fi ;;
  esac
done
FINAL=$(title)

echo "== result ($MODE) =="
echo "final title: $FINAL"
# Per clip, between the first sample at 3 s or later and the last one.
awk -v hz="$HZ" '
  $2 >= 3 { if (!($1 in a)) { a[$1] = $0 } b[$1] = $0 }
  END { for (p in a) { split(a[p], x); split(b[p], y); dt = y[9] - x[9]; if (dt <= 0) continue
      printf "clip %s: %.1f frames/s shown+dropped, %d dropped in %.0f s | browser %.0f%%  niri %.0f%% (of one core) | whole system busy %.0f%% of 4 cores\n",
        p, (y[3]-x[3])/dt, y[4]-x[4], dt, (y[5]-x[5])/hz/dt*100, (y[6]-x[6])/hz/dt*100, (1-(y[8]-x[8])/(y[7]-x[7]))*100 } }' "$OUT/samples"
if [ -n "$DRVLOG" ]; then   # read as the browser user: the file is that user's
  $AS_USER sh -c 'f=$1; [ -s "$f" ] || { echo "driver log: empty — Chromium never loaded the VA-API driver"; exit 0; }
    echo "driver log: $(grep -c -E "sending [0-9]+ bytes" "$f") pictures sent to the hardware, $(grep -c "ExportSurfaceHandle: surface=.*export_fd" "$f") surface exports, $(grep -c -i -E "fail|error" "$f") lines with fail/error"
    grep -E "CreateConfig|CreateContext: config" "$f" | sed "s/^[^]]*] /  /" | sort | uniq -c | head -6
    grep -E "ExportSurfaceHandle: (surface=.*export_fd|surface=.*flags)" "$f" | sed "s/^[^]]*] /  /; s/surface=0x[0-9a-f]* //; s/export_fd=[0-9]* //" | sort | uniq -c | sort -rn | head -4
    grep -E "assign_mpp_frame" "$f" | sed "s/.*\(copied=[0-9]*\).*/  frames \1/; s/.*zero-copy.*/  frames zero-copy/" | sort | uniq -c | head -4
    grep -i -E "fail|error" "$f" | sed "s/^[^]]*] /  /" | sort | uniq -c | head -5' sh "$DRVLOG" </dev/null
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
[ -n "$DRVLOG" ] && $AS_USER cat "$DRVLOG" > "$OUT/rk-vaapi.log" 2>/dev/null </dev/null   # a copy, written by root
case $PROFILE in /tmp/vaapi-test-profile.*) rm -rf "$PROFILE" ;; esac
echo "closed; logs in $OUT"
