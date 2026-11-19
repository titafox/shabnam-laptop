#!/bin/sh
# Builds the release binary and assembles "build/Shabnam Display.app".
#
#   scripts/bundle.sh
#   CODESIGN_IDENTITY="My Signing Identity" scripts/bundle.sh
#
# By default the app is signed ad hoc ("-"). An ad-hoc signature changes with every build, so
# macOS forgets the Screen Recording and Accessibility permissions after each rebuild; signing
# with a stable identity (any certificate in your keychain) avoids that.
set -eu
cd "$(dirname "$0")/.."

APP="build/Shabnam Display.app"
IDENTITY="${CODESIGN_IDENTITY:--}"

swift build -c release
BIN="$(swift build -c release --show-bin-path)/ShabnamDisplay"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/ShabnamDisplay"
cp Resources/Info.plist "$APP/Contents/Info.plist"
cp Resources/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
cp -R Resources/en.lproj Resources/zh-Hans.lproj "$APP/Contents/Resources/"
printf 'APPL????' > "$APP/Contents/PkgInfo"

codesign --force --sign "$IDENTITY" --identifier org.shabnam.display "$APP"
codesign --verify --strict "$APP"
echo "built $APP"
