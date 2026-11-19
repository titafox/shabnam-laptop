#!/bin/bash
# 电脑上测试协议核心：只编译三个纯 Java 文件和测试程序，不需要安卓 SDK。
set -euo pipefail
cd "$(dirname "$0")/.."
rm -rf test/out; mkdir -p test/out/classes
javac -source 8 -target 8 -Xlint:-options -nowarn -d test/out/classes src/LsJson.java src/LsCert.java src/LsCore.java test/LsTest.java
# macOS 的双栈套接字发不出 IPv4 组播，所以测试时让 JVM 只用 IPv4（安卓/Linux 没有这个问题）
exec java -Djava.net.preferIPv4Stack=true -cp test/out/classes local.cloud.launcher.LsTest "${@:-all}"
