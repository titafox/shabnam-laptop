#!/bin/sh
# Cross-builds the browser's GL stand-in on the Mac (needs zig). Output: ./libEGL.so.1 (aarch64, glibc).
set -e
cd "$(dirname "$0")"
T=aarch64-linux-gnu.2.28
mkdir -p stub
echo 'void shabnam_stub(void){}' > stub/empty.c
# an empty library that only carries the name libmali.so.1, so the result depends on the real one at run time
zig cc -target $T -shared -fPIC -Wl,-soname,libmali.so.1 -o stub/libmali.so.1 stub/empty.c
zig cc -target $T -shared -fPIC -O2 -Wall -fvisibility=hidden -Wl,-soname,libEGL.so.1 -Wl,-Bsymbolic -Wl,--no-as-needed \
  -o libEGL.so.1 mali-finish.c stub/libmali.so.1 -ldl
rm -rf stub
