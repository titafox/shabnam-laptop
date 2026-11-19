#!/bin/sh
mkdir -p /run/user0 && chmod 700 /run/user0
# Android blanks the screen when its interface stops: the kernel then keeps the backlight PWM at 0
# no matter what brightness is written, until the framebuffer is unblanked again.
(sleep 6   # after the compositor has lit the display pipeline
 echo 0 > /sys/class/graphics/fb0/blank   # clear the kernel's "screen blanked" flag, which holds the lamp off
 echo 160 > /sys/class/backlight/backlight/brightness) &
export LD_LIBRARY_PATH=/opt/mali/mali:/opt/mali LIBSEAT_BACKEND=noop RUST_BACKTRACE=1
setsid niri-mali -c /root/niri.kdl >/tmp/niri.log 2>&1 &
