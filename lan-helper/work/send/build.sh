#!/bin/bash
# 一条命令构建「开放桌面」ChromeOS 风格版：编译 → 打包 → 对齐 → 签名。
# 控制通道源码（AgentService / ApkProvider）直接取自 ../../desktop/，不复制。
set -euo pipefail
cd "$(dirname "$0")"
SDK=${ANDROID_SDK:-/opt/homebrew/share/android-commandlinetools}
BT=$SDK/build-tools/35.0.0
JAR=$SDK/platforms/android-34/android.jar
KEYS=../../.install-keys
OUT=build
rm -rf "$OUT"; mkdir -p "$OUT/classes" "$OUT/dex"
"$BT/aapt2" compile --dir res -o "$OUT/res.zip"
"$BT/aapt2" link -o "$OUT/base.apk" -I "$JAR" --manifest AndroidManifest.xml "$OUT/res.zip"
AGENT=../../desktop/AgentService.java
APK=open-desktop.apk
# 模拟器测试用：TEST=1 时把控制通道地址换成空地址，绝不能连到真实通道。测试包不得推到真机。
if [ "${TEST:-}" = "1" ]; then
  mkdir -p "$OUT/gen"
  sed 's#static final String BASE = "[^"]*"#static final String BASE = "http://127.0.0.1:9/"#' "$AGENT" > "$OUT/gen/AgentService.java"
  grep -q '127.0.0.1:9' "$OUT/gen/AgentService.java" || { echo "未能替换通道地址"; exit 1; }
  AGENT=$OUT/gen/AgentService.java
  APK=TEST-ONLY.apk
fi
javac -source 8 -target 8 -Xlint:-options -nowarn -cp "$JAR" -d "$OUT/classes" \
  src/*.java "$AGENT" ../../desktop/ApkProvider.java
"$BT/d8" --release --min-api 23 --lib "$JAR" --output "$OUT/dex" $(find "$OUT/classes" -name '*.class')
zip -qj "$OUT/base.apk" "$OUT/dex/classes.dex"
"$BT/zipalign" -f 4 "$OUT/base.apk" "$OUT/aligned.apk"
# 默认用 AOSP 公开的平台测试钥匙签名：设备上的通用系统镜像用的就是它，
# 这样桌面才能拿到系统窗口/任务管理权限（常驻底栏）。PLATFORM=0 时退回原来的自有钥匙。
if [ "${PLATFORM:-1}" = "1" ]; then
  "$BT/apksigner" sign --key ../../.platform-keys/platform.pk8 --cert ../../.platform-keys/platform.x509.pem --out "$OUT/$APK" "$OUT/aligned.apk"
else
  PK8=$(mktemp); trap 'rm -f "$PK8"' EXIT
  openssl pkcs8 -topk8 -nocrypt -inform PEM -outform DER -in "$KEYS/apk-signing.pem" -out "$PK8"
  "$BT/apksigner" sign --key "$PK8" --cert "$KEYS/apk-cert.pem" --out "$OUT/$APK" "$OUT/aligned.apk"
fi
"$BT/apksigner" verify --print-certs "$OUT/$APK" | grep -E "CN=|SHA-256" | head -2
ls -l "$OUT/$APK"
