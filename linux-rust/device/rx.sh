#!/bin/sh
# start the second-screen receiver (stops the compositor first: only one program can own the display)
p=$(pidof niri-mali); [ -n "$p" ] && kill $p && sleep 1
p=$(pidof subscreen-rx); [ -n "$p" ] && kill $p && sleep 0.5
export LD_LIBRARY_PATH=/opt/mpp2/usr/lib/aarch64-linux-gnu
setsid /usr/local/bin/subscreen-rx "$@" >/var/log/shabnam/rx.log 2>&1 &
sleep 1.5; cat /var/log/shabnam/rx.log
