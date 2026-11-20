#!/bin/sh
# Browser flicker regression test — run on the Mac with the laptop on adb.
#   ./run-test.sh [NAME] [EXTRA_CHROMIUM_FLAGS] [RUNS] [ENV]
#   ./run-test.sh now                                # the installed browser, 2 windows (about 75 screenshots)
#   ./run-test.sh off "" 2 SHABNAM_GL_FINISH=0       # the same without the fix: expect about 5 % broken frames
#   ./run-test.sh sw --disable-gpu                   # software compositing for comparison
# Each run opens one test browser window on the laptop for about 23 s (own throw-away profile, the owner's
# browser is not touched), scrolls and cross-fades a page of full-width colour stripes that also repaints
# and navigates, and takes a burst of screenshots. score.py then counts screenshots in which a pixel row
# of the page is not one colour or the stripes are not 60 px apart: "glitched N/M". Expect 0.
# Also printed: fps (requestAnimationFrame rate read from the window title) and CPU use.
# Note: each screenshot also lands in the laptop's clipboard (niri does that).
# Needs: adb, python3 with numpy and pillow (PYTHON=/path/to/python to choose one).
# Results are kept in $OUT (default /tmp/shabnam-flicker).
cd "$(dirname "$0")" || exit 1
ADB="adb ${SERIAL:+-s $SERIAL}"; PY=${PYTHON:-python3}; OUT=${OUT:-/tmp/shabnam-flicker}
NAME=${1:-now}; R=${3:-2}; DIRS=""
for f in a.html b.html inner.py device-run.sh; do $ADB push $f /data/local/tmp/fl-$f >/dev/null || exit 1; done
for i in $(seq 1 "$R"); do
  N=$NAME-$i; rm -rf "$OUT/$N"; mkdir -p "$OUT/$N"
  $ADB shell "su 0 sh /data/local/tmp/fl-device-run.sh '$N' '$2' 45 '$4'" | tail -1 | "$PY" -c "
import json,sys
r=json.loads(sys.stdin.read()); print({k:r.get(k) for k in ('name','fps','cpu_total_pct_of_4cores','cpu_chromium_pct_1core','cpu_niri_pct_1core','shots','error','leftover')})"
  $ADB pull /data/local/tmp/fl-$N.tar "$OUT/$N.tar" >/dev/null && tar xf "$OUT/$N.tar" -C "$OUT/$N" && rm "$OUT/$N.tar"
  $ADB shell "rm -f /data/local/tmp/fl-$N.tar"
  DIRS="$DIRS $OUT/$N"
done
$ADB shell "rm -f /data/local/tmp/fl-a.html /data/local/tmp/fl-b.html /data/local/tmp/fl-inner.py /data/local/tmp/fl-device-run.sh"
"$PY" score.py $DIRS
