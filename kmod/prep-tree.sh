#!/bin/bash
# Step 1 of build-extra: re-create the kernel tree exactly as build-on-server.sh did (same commit, same .config).
set -euo pipefail
W=/root/rk-kmod; cd $W
COMMIT=f39b590ea11da2d897644e42119ba5099b2eefc8
if [ ! -d kernel ]; then
  git init -q kernel; cd kernel
  git remote add origin https://github.com/rockchip-linux/kernel
  git fetch -q --depth 1 origin $COMMIT
  git checkout -q FETCH_HEAD; cd ..
fi
cd kernel; [ "$(git rev-parse HEAD)" = "$COMMIT" ]
export ARCH=arm64 CROSS_COMPILE=aarch64-linux-gnu-
cp $W/device.config .config
scripts/config -d USB_DWC3_HOST -e USB_DWC3_DUAL_ROLE      # same .config as the dwc3 build; nothing enabled for the new modules
touch .scmversion
MK="nice make CC=${CROSS_COMPILE}gcc"
$MK olddefconfig && $MK -j8 modules_prepare
cp $W/Module.symvers ./Module.symvers
echo PREP_DONE; git rev-parse HEAD; head -5 Makefile | tr '\n' ' '; du -sh $W/kernel; df -h / | tail -1
