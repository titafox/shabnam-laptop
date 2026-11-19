#!/bin/bash
# UNTESTED DRAFT - never executed (build server was unreachable). Run on the x86_64 Linux build host.
# Needs next to it: device.config, Module.symvers (both produced from the device / boot.img).
set -euo pipefail
W=/root/rk-kmod; HERE=$(cd "$(dirname "$0")" && pwd); mkdir -p $W; cd $W
# deps (install only, no upgrades): apt-get install -y --no-install-recommends gcc-aarch64-linux-gnu bc bison flex libssl-dev git make
[ -d kernel ] || git clone --depth 1 -b develop-4.19 https://github.com/rockchip-linux/kernel kernel   # head f39b590ea11d, 4.19.232
cd kernel; export ARCH=arm64 CROSS_COMPILE=aarch64-linux-gnu-
cp $HERE/device.config .config
scripts/config -d USB_DWC3_HOST -e USB_DWC3_DUAL_ROLE      # only touches dwc3-private code/structs
touch .scmversion                                         # no "+" in vermagic
MK="nice make CC=${CROSS_COMPILE}gcc"   # bypass scripts/gcc-wrapper.py (python2)
$MK olddefconfig && $MK -j8 modules_prepare
diff <(grep -v DWC3 $HERE/device.config | grep '^CONFIG' | sort) <(grep -v DWC3 .config | grep '^CONFIG' | sort) || echo "WARNING: config drift, review (struct layout risk)"
cp $HERE/Module.symvers ./Module.symvers                  # CRCs of the RUNNING kernel -> modpost writes matching __versions
M=$W/dwc3-gadget-otg; rm -rf $M; mkdir -p $M
cp drivers/usb/dwc3/{core.c,core.h,host.c,gadget.c,gadget.h,ep0.c,drd.c,io.h,debug.h,debugfs.c} $M/
cd $M
sed -i 's/\.name\t*= "dwc3",/.name\t= "dwc3-gadget-otg",/; /MODULE_ALIAS("platform:dwc3")/d' core.c
sed -i '/^EXPORT_SYMBOL_GPL(dwc3_host_\(init\|exit\));/d' host.c   # vmlinux already exports these
cat > trace.h <<'T'
/* tracepoints stubbed: built-in dwc3 already registered the same trace events */
#ifndef __DWC3_TRACE_STUB_H
#define __DWC3_TRACE_STUB_H
#define trace_dwc3_readl(...) do {} while (0)
#define trace_dwc3_writel(...) do {} while (0)
#define trace_dwc3_event(...) do {} while (0)
#define trace_dwc3_ctrl_req(...) do {} while (0)
#define trace_dwc3_alloc_request(...) do {} while (0)
#define trace_dwc3_free_request(...) do {} while (0)
#define trace_dwc3_ep_queue(...) do {} while (0)
#define trace_dwc3_ep_dequeue(...) do {} while (0)
#define trace_dwc3_gadget_giveback(...) do {} while (0)
#define trace_dwc3_gadget_generic_cmd(...) do {} while (0)
#define trace_dwc3_gadget_ep_cmd(...) do {} while (0)
#define trace_dwc3_prepare_trb(...) do {} while (0)
#define trace_dwc3_complete_trb(...) do {} while (0)
#define trace_dwc3_gadget_ep_enable(...) do {} while (0)
#define trace_dwc3_gadget_ep_disable(...) do {} while (0)
#endif
T
cat > Makefile <<'T'
obj-m := dwc3_gadget_otg.o
dwc3_gadget_otg-y := core.o host.o gadget.o ep0.o drd.o debugfs.o
ccflags-y := -Wno-unused-variable -Wno-unused-but-set-variable
T
$MK -C $W/kernel M=$M -j8 modules
modinfo dwc3_gadget_otg.ko | grep -E 'vermagic|name'      # must be: 4.19.232 SMP preempt mod_unload modversions aarch64
aarch64-linux-gnu-nm -u dwc3_gadget_otg.ko | awk '{print $2}' | while read s; do grep -qP "\t$s\t" $W/kernel/Module.symvers || echo "UNRESOLVED (not exported by running kernel): $s"; done
