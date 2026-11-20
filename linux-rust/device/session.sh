#!/bin/sh
# session.sh — runs inside the Void tree and owns the screen. Two modes, one at a time:
#   desktop : the niri desktop with the Shabnam shelf.   Mod+Esc switches to mac mode.
#   mac     : second screen for the Mac.     Mod+Esc again switches back to the desktop.
export LD_LIBRARY_PATH=/opt/mali/mali:/opt/mali:/opt/mpp2/usr/lib/aarch64-linux-gnu
export LIBSEAT_BACKEND=noop RUST_BACKTRACE=1
export XDG_RUNTIME_DIR=/run/user0 HOME=/root PATH=/usr/local/bin:/usr/bin:/usr/sbin:/bin:/sbin
export LANG=zh_CN.UTF-8
# Everything the session creates (sockets above all) is for root only unless opened on purpose:
# the browser runs as the unprivileged user "shabnam" and can look into /run/user0.
umask 022
# Logs live in a directory only root can enter; /tmp is shared with the browser's user.
mkdir -p /var/log/shabnam && chmod 700 /var/log/shabnam
echo 1 > /proc/sys/fs/protected_symlinks 2>/dev/null; echo 1 > /proc/sys/fs/protected_hardlinks 2>/dev/null
mkdir -p /run/user0 && chmod 700 /run/user0
# Compressed swap in memory: with 4 GB and no swap, many open browser tabs stall the machine
# (returning to an old tab took 12 s without it, 0.2 s with it).
if ! grep -q zram0 /proc/swaps; then
  echo lz4 > /sys/block/zram0/comp_algorithm
  echo 2G > /sys/block/zram0/disksize
  mkswap /dev/block/zram0 >/dev/null && swapon -p 100 /dev/block/zram0
fi 2>/dev/null
echo 100 > /proc/sys/vm/swappiness; echo 0 > /proc/sys/vm/page-cluster
pidof udevd >/dev/null || { udevd --daemon; udevadm trigger --action=add --subsystem-match=input --subsystem-match=drm; udevadm settle; }
# Hands out the 10.55.0.x address the Mac side uses to recognise the cable. The cable's network
# port appears a few seconds after start-up and is re-created when the USB link is re-bound, so
# dnsmasq is told to follow interfaces as they come and go instead of binding once.
pidof dnsmasq >/dev/null || dnsmasq --interface=usb0 --bind-dynamic --port=0 \
  --dhcp-range=10.55.0.2,10.55.0.9,255.255.255.0,12h --dhcp-option=3 --dhcp-option=6 \
  --dhcp-leasefile=/var/log/shabnam/usb0.leases >/var/log/shabnam/dnsmasq.log 2>&1

# Join the saved Wi-Fi network (see wifi-setup); the desktop does not wait for it.
[ -s /etc/wpa_supplicant/wpa_supplicant-wlan0.conf ] && (wifi-up >/var/log/shabnam/wifi-up.log 2>&1 &)

light() {  # the kernel keeps the backlight off until the framebuffer is unblanked
  (sleep 5; echo 0 > /sys/class/graphics/fb0/blank; b=/sys/class/backlight/backlight
   [ "$(cat $b/brightness)" -lt 20 ] && echo 160 > $b/brightness; echo 0 > $b/bl_power) &
}
# The start-up picture owns the screen until the first real program is about to take it.
end_splash() { for p in $(pidof shabnam-splash); do kill $p; done; }
# The Mac app's controls (battery, brightness, volume, screen, power) while this laptop is on its
# own desktop: the second-screen receiver's control port alone, on the cable only. It retries by
# itself until the cable's network port exists and follows it when it is re-created. Second-screen
# mode serves the same port itself, so this one is stopped first — and waited for.
CTL=/run/shabnam-control.pid
control_running() {
  ctl=$(cat $CTL 2>/dev/null) && [ -n "$ctl" ] &&
    case "$(tr '\0' ' ' < /proc/$ctl/cmdline 2>/dev/null)" in *"subscreen-rx control"*) true ;; *) false ;; esac
}
control_start() {
  control_running && return
  subscreen-rx control >/var/log/shabnam/control.log 2>&1 &
  echo $! > $CTL
}
control_stop() {
  if control_running; then
    kill "$ctl" 2>/dev/null; wait "$ctl" 2>/dev/null
    n=0; while [ -d /proc/$ctl ] && [ $n -lt 30 ]; do sleep 0.1; n=$((n+1)); done  # not our child: left by an earlier run
  fi
  rm -f $CTL
}
shabnam-timesync start >/var/log/shabnam/timesync.log 2>&1   # network time (ntpd-rs)
shabnam-lid start >/var/log/shabnam/lid.log 2>&1             # lid closed = backlight off
echo desktop > /run/mode
fails=0
while true; do
  start=$(date +%s)
  light
  if [ "$(cat /run/mode)" = mac ]; then
    end_splash
    shabnam-audio stop >/dev/null 2>&1   # the second-screen receiver plays sound itself
    control_stop
    subscreen-rx >/var/log/shabnam/rx.log 2>&1
    # The Mac app can ask for power off or restart from here too (control channel).
    case "$(cat /run/mode)" in poweroff|reboot) sync; exit 0 ;; esac
    echo desktop > /run/mode
  else
    end_splash
    shabnam-audio start >/var/log/shabnam/audio.log 2>&1
    control_start
    niri-mali -c /root/niri.kdl >/var/log/shabnam/niri.log 2>&1
    # The quick panel's power buttons leave a word here and end the desktop.
    # (Or the Mac app asked for the same over the control channel.)
    case "$(cat /run/mode)" in poweroff|reboot) control_stop; shabnam-audio stop >/dev/null 2>&1; sync; exit 0 ;; esac
  fi
  # Something that dies immediately, over and over, is broken: stop instead of spinning.
  if [ $(( $(date +%s) - start )) -lt 5 ]; then fails=$((fails+1)); else fails=0; fi
  [ $fails -ge 5 ] && { echo "giving up: session keeps exiting" >&2; exit 1; }
  sleep 1
done
