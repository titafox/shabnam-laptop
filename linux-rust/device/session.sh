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
mkdir -p /run/user0 && chmod 700 /run/user0
pidof udevd >/dev/null || { udevd --daemon; udevadm trigger --action=add --subsystem-match=input --subsystem-match=drm; udevadm settle; }
# Hands out the 10.55.0.x address the Mac side uses to recognise the cable.
# The cable's network port appears a few seconds after start-up, so wait for it.
(until ip link show usb0 >/dev/null 2>&1; do sleep 2; done
 pidof dnsmasq >/dev/null || dnsmasq --interface=usb0 --bind-interfaces --port=0 \
  --dhcp-range=10.55.0.2,10.55.0.9,255.255.255.0,12h --dhcp-option=3 --dhcp-option=6 --dhcp-leasefile=/tmp/usb0.leases) >/tmp/dnsmasq.log 2>&1 &

# Join the saved Wi-Fi network (see wifi-setup); the desktop does not wait for it.
[ -s /etc/wpa_supplicant/wpa_supplicant-wlan0.conf ] && (wifi-up >/tmp/wifi-up.log 2>&1 &)

light() {  # the kernel keeps the backlight off until the framebuffer is unblanked
  (sleep 5; echo 0 > /sys/class/graphics/fb0/blank; b=/sys/class/backlight/backlight
   [ "$(cat $b/brightness)" -lt 20 ] && echo 160 > $b/brightness; echo 0 > $b/bl_power) &
}
# The start-up picture owns the screen until the first real program is about to take it.
end_splash() { for p in $(pidof shabnam-splash); do kill $p; done; }
shabnam-timesync start >/tmp/timesync.log 2>&1   # network time (ntpd-rs)
shabnam-lid start >/tmp/lid.log 2>&1             # lid closed = backlight off
echo desktop > /run/mode
fails=0
while true; do
  start=$(date +%s)
  light
  if [ "$(cat /run/mode)" = mac ]; then
    end_splash
    shabnam-audio stop >/dev/null 2>&1   # the second-screen receiver plays sound itself
    subscreen-rx >/tmp/rx.log 2>&1
    echo desktop > /run/mode
  else
    end_splash
    shabnam-audio start >/tmp/audio.log 2>&1
    niri-mali -c /root/niri.kdl >/tmp/niri.log 2>&1
    # The quick panel's power buttons leave a word here and end the desktop.
    case "$(cat /run/mode)" in poweroff|reboot) shabnam-audio stop >/dev/null 2>&1; sync; exit 0 ;; esac
  fi
  # Something that dies immediately, over and over, is broken: stop instead of spinning.
  if [ $(( $(date +%s) - start )) -lt 5 ]; then fails=$((fails+1)); else fails=0; fi
  [ $fails -ge 5 ] && { echo "giving up: session keeps exiting" >&2; exit 1; }
  sleep 1
done
