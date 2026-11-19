#!/bin/bash
# Struct-layout cross-check: recompute CRCs with genksyms from the tree for objects that export what the new modules use.
set -euo pipefail
W=/root/rk-kmod; K=$W/kernel; cd $K
export ARCH=arm64 CROSS_COMPILE=aarch64-linux-gnu-
OBJS="net/ipv4/tcp_cong.o net/ipv4/tcp_output.o net/ipv4/tcp_input.o net/ipv4/tcp_rate.o net/ipv4/tcp.o net/ipv4/tcp_ipv4.o net/ipv4/tcp_minisocks.o net/ipv4/tcp_timer.o net/ipv4/inet_connection_sock.o lib/win_minmax.o
 net/sched/sch_api.o net/sched/sch_generic.o net/sched/sch_fifo.o net/core/gen_stats.o net/core/skbuff.o net/core/sock.o net/core/dev.o net/core/rtnetlink.o net/core/flow_dissector.o lib/nlattr.o lib/rbtree.o
 drivers/usb/gadget/function/u_ether.o drivers/usb/gadget/functions.o drivers/usb/gadget/composite.o drivers/usb/gadget/config.o drivers/usb/gadget/usbstring.o drivers/usb/gadget/epautoconf.o drivers/usb/gadget/configfs.o drivers/usb/gadget/udc/core.o
 fs/configfs/item.o kernel/time/hrtimer.o net/ethernet/eth.o kernel/module.o"
nice make CC=${CROSS_COMPILE}gcc KBUILD_SYMTYPES=1 -j8 $OBJS > $W/layout-build.log 2>&1 || { tail -30 $W/layout-build.log; exit 1; }
python3 $W/closure.py $W/Module.symvers $W/tree-crcs-extra.txt $(for o in $OBJS; do echo ${o%.o}.symtypes; done)
