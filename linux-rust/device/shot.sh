#!/bin/sh
export NIRI_SOCKET=$(ls /run/user0/niri.*.sock | head -1)
mkdir -p /root/Pictures/Screenshots; rm -f /root/Pictures/Screenshots/*
niri-mali msg windows 2>&1 | head -8
niri-mali msg action screenshot-screen 2>&1 | head -3
sleep 2; ls -la /root/Pictures/Screenshots/
