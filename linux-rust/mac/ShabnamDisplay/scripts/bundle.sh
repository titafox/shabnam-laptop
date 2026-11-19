#!/bin/sh
# Builds the release binary and assembles "build/Shabnam Display.app".
#
#   scripts/bundle.sh
#
# Signing identity, first match wins:
#   1. the CODESIGN_IDENTITY environment variable
#   2. the first line of .codesign-identity in the project root (git-ignored, machine-local)
#   3. ad hoc ("-")
# The value is anything `codesign --sign` accepts: a certificate's SHA-1 hash or its name.
# `security find-identity -v -p codesigning` lists what the keychain offers.
#
# Why it matters: macOS ties the privacy permissions (Screen Recording, Accessibility, System
# Audio Recording) to the code signature. An ad-hoc signature changes with every build, so the
# permissions have to be granted again after each rebuild; with a stable identity they survive.
set -eu
cd "$(dirname "$0")/.."

APP="build/Shabnam Display.app"
IDENTITY="${CODESIGN_IDENTITY:-}"
if [ -z "$IDENTITY" ] && [ -f .codesign-identity ]; then
  IDENTITY="$(sed -n '1p' .codesign-identity | sed 's/^[[:space:]]*//; s/[[:space:]]*$//')"
fi
[ -n "$IDENTITY" ] || IDENTITY="-"

swift build -c release
BIN="$(swift build -c release --show-bin-path)/ShabnamDisplay"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/ShabnamDisplay"
cp Resources/Info.plist "$APP/Contents/Info.plist"
# The version comes from the VERSION file; the build number from BUILD_NUMBER, else the number
# of commits, else 1.
VERSION="$(sed -n '1p' VERSION | tr -d '[:space:]')"
BUILD="${BUILD_NUMBER:-$(git rev-list --count HEAD 2>/dev/null || echo 1)}"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $VERSION" -c "Set :CFBundleVersion $BUILD" "$APP/Contents/Info.plist"
cp Resources/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
cp -R Resources/en.lproj Resources/zh-Hans.lproj "$APP/Contents/Resources/"
printf 'APPL????' > "$APP/Contents/PkgInfo"

codesign --force --sign "$IDENTITY" --identifier org.shabnam.display "$APP"
codesign --verify --strict "$APP"
if [ "$IDENTITY" = "-" ]; then
  echo "built $APP $VERSION ($BUILD) (ad-hoc signature: permissions must be granted again after every rebuild)"
else
  echo "built $APP $VERSION ($BUILD) (signed with the configured identity)"
fi
