#!/bin/bash
set -eu
export PATH="/opt/homebrew/bin:$PATH"
cd "$(dirname "$0")"
adb start-server
case "${1:-check}" in
check)
  adb devices -l
  if ! adb get-state >/dev/null; then
    echo '需要一台状态为 device 的终端。若显示 unauthorized，请在云笔电确认 USB 调试授权。'
    exit 1
  fi
  adb shell getprop ro.product.model
  adb shell cmd package query-activities --brief -a android.intent.action.MAIN -c android.intent.category.HOME
  ;;
shell) exec adb shell ;;
desktop)
  adb get-state >/dev/null
  adb install -r lan-helper/dist/desktop.apk
  adb shell am start -n local.cloud.desktop/.Main
  ;;
home) exec adb shell am start -a android.settings.HOME_SETTINGS ;;
*) echo '支持 check、shell、desktop、home'; exit 2 ;;
esac
