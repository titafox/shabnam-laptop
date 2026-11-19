#!/bin/sh
# mirror.sh — run on the Mac. Captures the main screen, encodes it with the Mac's hardware
# H.264 encoder and streams it over the USB cable to subscreen-rx on the laptop.
# Keeps the Mac screen's shape (black bars left and right) and favours sharp text over bitrate.
# Prototype: mirrors the existing screen (no virtual display yet). Stop with Ctrl-C.
# The terminal running this needs the macOS "Screen Recording" permission.
HOST=${1:-169.254.1.1}
SCREEN=${SCREEN:-2}   # index from: ffmpeg -f avfoundation -list_devices true -i ""
exec ffmpeg -hide_banner -loglevel warning \
  -f avfoundation -capture_cursor 1 -framerate 60 -pixel_format nv12 -i "$SCREEN:none" \
  -vf "scale=-2:1080:flags=lanczos,pad=1920:1080:(ow-iw)/2:0" -fps_mode passthrough \
  -c:v h264_videotoolbox -realtime 1 -prio_speed 1 -profile:v high -b:v 40M -bf 0 -g 600 \
  -flags +low_delay -bsf:v h264_mp4toannexb -flush_packets 1 \
  -f h264 "tcp://$HOST:7000?tcp_nodelay=1"
