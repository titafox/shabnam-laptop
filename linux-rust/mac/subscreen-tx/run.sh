#!/bin/sh
# run.sh — keeps the second screen available: waits for the laptop on the cable, runs until it is
# unplugged (the extra screen disappears and windows return to the Mac), then waits again.
cd "$(dirname "$0")" || exit 1
while true; do
  ./subscreen-tx "$@"
  sleep 1
done
