#!/bin/sh
# start the device manager only for input and display devices, then restart the compositor
pidof udevd >/dev/null || udevd --daemon
udevadm trigger --action=add --subsystem-match=input --subsystem-match=drm
udevadm settle --timeout=10
udevadm info /dev/input/event2 | grep -E "ID_INPUT|ID_SEAT" | head -5
udevadm info /dev/input/event3 | grep -E "ID_INPUT" | head -4
