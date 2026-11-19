#!/bin/bash
# Builds tcp_bbr.ko, sch_fq.ko, usb_f_ncm.ko, usb_f_ecm.ko as out-of-tree copies of the tree's own sources.
# Run after prep-tree.sh (same tree/commit/.config as the dwc3 build; NO config option is enabled for these modules).
set -euo pipefail
W=/root/rk-kmod; K=$W/kernel; M=$W/extra-mods
export ARCH=arm64 CROSS_COMPILE=aarch64-linux-gnu-
MK="nice make CC=${CROSS_COMPILE}gcc"
cd $K
cmp $W/Module.symvers ./Module.symvers                    # running kernel's CRCs are in place
rm -rf $M; mkdir -p $M
cp net/ipv4/tcp_bbr.c net/sched/sch_fq.c $M/
F=drivers/usb/gadget/function
cp $F/f_ncm.c $F/f_ecm.c $F/u_ether.h $F/u_ether_configfs.h $F/u_ncm.h $F/u_ecm.h $M/   # headers only; u_ether.c is NOT copied (built into vmlinux)
cat > $M/Makefile <<'T'
obj-m := tcp_bbr.o sch_fq.o usb_f_ncm.o usb_f_ecm.o
usb_f_ncm-y := f_ncm.o
usb_f_ecm-y := f_ecm.o
T
( cd $M && sha256sum *.c *.h > SOURCES.sha256 )
$MK -C $K M=$M -j8 modules
cd $M
for m in tcp_bbr sch_fq usb_f_ncm usb_f_ecm; do
  echo "=== $m.ko"; sha256sum $m.ko
  modinfo $m.ko | grep -E '^(name|vermagic|alias|depends|license)'
  aarch64-linux-gnu-nm -u $m.ko | awk '{print $2}' | sort > $m.undef
  modprobe --dump-modversions $m.ko | awk '{print $1"\t"$2}' | sort -k2 > $m.versions
  nu=$(wc -l < $m.undef); nv=$(wc -l < $m.versions); miss=0; bad=0
  while read s; do grep -qP "\t$s\tvmlinux\t" $W/Module.symvers || { echo "UNRESOLVED (not exported by running kernel): $s"; miss=$((miss+1)); }; done < $m.undef
  while IFS=$'\t' read c s; do
    want=$(awk -F'\t' -v s="$s" '$2==s{print $1}' $W/Module.symvers)
    [ "$((c))" = "$((want))" ] || { echo "CRC MISMATCH $s: ko=$c running=$want"; bad=$((bad+1)); }
  done < $m.versions
  echo "undefined=$nu unresolved=$miss ; __versions entries=$nv mismatches=$bad"
done
