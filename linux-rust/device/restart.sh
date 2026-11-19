#!/bin/sh
# runs inside the Void tree: restart the compositor with the binary staged in /root/niri-new (if any)
pkill -x niri-mali; pkill -x alacritty; sleep 1
[ -f /root/niri-new ] && mv /root/niri-new /usr/local/bin/niri-mali && chmod 755 /usr/local/bin/niri-mali
/root/start-niri.sh
sleep ${1:-9}
echo "PIDS niri=$(pidof niri-mali) alacritty=$(pidof alacritty)"
