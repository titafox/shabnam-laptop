# dwc3 gadget module for EC-NB01-CE (RK3568, Android 11, kernel 4.19.232)

STATUS (2026-11-19): `dwc3_gadget_otg.ko` is **built and statically verified**; it has **never been loaded**
on the device (device dropped off the network before `insmod` could run). Everything under
"Load / bind" is untested.

## Files
- `dwc3_gadget_otg.ko` — the module. sha256 `33de142683f0b492a3026fdec6c1022ff28f58f526c5fb718ab48d160d50d3a9`
- `device.config` — `/proc/config.gz` from the device.
- `Module.symvers` — CRCs of the *running* kernel (6120 EXPORT_SYMBOL + 5699 EXPORT_SYMBOL_GPL).
- `extract_crcs.py`, `kallsyms-table-addrs.txt` — how Module.symvers was produced.
- `build-on-server.sh` — the exact build script that produced the .ko; `build.log` — its output.
- `tree-crcs.txt` — 284 CRCs recomputed from the source tree (layout cross-check, see below).

## Source
https://github.com/rockchip-linux/kernel branch `develop-4.19`,
commit `f39b590ea11da2d897644e42119ba5099b2eefc8` (2022-08-04, Makefile = 4.19.232). Shallow clone.

## Config changes
`.config` = `device.config`, then only: `CONFIG_USB_DWC3_HOST` off, `CONFIG_USB_DWC3_DUAL_ROLE=y`
(affects dwc3-private code only). `make olddefconfig` additionally dropped vendor-only options that do
not exist in this tree (EXFAT_*, LEDS_WORKING, LEDS_TRIGGER_SLEEP, UWE5621/SPRDWL/RTL8822ES wifi,
TTY_OVERY_SDIO, USB_KEYBOARD_GPIO, WCN_BSP_DRIVER_BUILDIN) and changed compiler-probe values
(GCC_VERSION 60301 -> 120200, CC_HAS_* additions). Full diff is in `build.log`.

## Build (on root@185.138.186.149 via jump host, in /root/rk-kmod/)
```
apt-get install -y --no-install-recommends gcc-aarch64-linux-gnu bc bison flex   # Debian gcc 12.2.0
bash build-on-server.sh        # needs device.config + Module.symvers next to it
```
The script: clone, copy config, `scripts/config -d USB_DWC3_HOST -e USB_DWC3_DUAL_ROLE`, `touch .scmversion`,
`make CC=aarch64-linux-gnu-gcc olddefconfig modules_prepare` (CC= bypasses the tree's python2
`scripts/gcc-wrapper.py`), then builds an out-of-tree copy of `drivers/usb/dwc3/` as `dwc3_gadget_otg.ko` with:
- platform driver renamed `dwc3` -> `dwc3-gadget-otg` (of_match `snps,dwc3` kept), `MODULE_ALIAS("platform:dwc3")` removed;
- `EXPORT_SYMBOL_GPL(dwc3_host_init/exit)` removed (vmlinux already exports them);
- tracepoints stubbed out (`trace.h` replaced, `trace.c` not built) because the built-in dwc3 already registered the same trace events.

Compiler differs from the stock kernel (Linaro GCC 6.3.1; its download is no longer available) — not load-tested.

## How CRCs were fixed
MODVERSIONS is on. `Module.symvers` was extracted from boot.img (arm64 `Image` at offset 2048): `__ksymtab`/`__ksymtab_gpl`
are PREL32 8-byte entries, `__kcrctab*` are u32; table addresses came from `/proc/kallsyms` (kptr_restrict temporarily 0).
It was copied into the kernel tree root before the module build, so modpost wrote the running kernel's CRCs into `__versions`.
Checks done:
- extraction validated against vendor `sprdbt_tty.ko`: 64/64 `__versions` entries match;
- our module: 134/134 `__versions` entries match Module.symvers, every undefined symbol (133) is exported by the running kernel;
- vermagic `4.19.232 SMP preempt mod_unload modversions aarch64` (identical to vendor modules);
- struct-layout cross-check: 284 CRCs recomputed by genksyms from this tree (udc core, composite, gadget config, phy core,
  usb phy, extcon, usb common, usb roles, platform, driver core, runtime PM, reset, `module_layout`) — all 284 equal the running kernel's.

## Hardware mapping (read from the device)
- OTG controller `fcc00000.dwc3` <-> HUSB311 at i2c `0-004e` = **typec port0**. Its buses usb5/usb6 were empty.
- **typec port1** (`1-004e`) -> hub 05e3:0608 on EHCI `fd880000.usb` (same hub as keyboard 1018:1007): host-only, can never be a gadget.
  On 2026-11-19 the Mac cable was in port1 (Mac enumerated on the laptop as 05ac:1905 at 2-1.4).
- Camera 1bcf:2ced is on EHCI `fd800000.usb`.

## Load / bind (UNTESTED; as root, Android 11 only, cable in port0)
Re-check first that usb5/usb6 are empty (`ls /sys/bus/usb/devices/ | grep -E '^[56]-'` prints nothing).
```
insmod /data/local/tmp/kmod/dwc3_gadget_otg.ko
echo fcc00000.dwc3 > /sys/bus/platform/drivers/dwc3/unbind
echo dwc3-gadget-otg > /sys/devices/platform/usbdrd/fcc00000.dwc3/driver_override
echo fcc00000.dwc3 > /sys/bus/platform/drivers/dwc3-gadget-otg/bind
echo peripheral > /sys/devices/platform/fe8a0000.usb2-phy/otg_mode
ls /sys/class/udc/                      # expect fcc00000.dwc3
setprop sys.usb.controller fcc00000.dwc3
echo fcc00000.dwc3 > /config/usb_gadget/g1/UDC
```
If the Type-C partner is attached but the port stays host: `echo device > /sys/class/typec/port0/data_role` (guess, unverified).

Revert: `echo none > /config/usb_gadget/g1/UDC`; `echo fcc00000.dwc3 > /sys/bus/platform/drivers/dwc3-gadget-otg/unbind`;
`echo > .../fcc00000.dwc3/driver_override`; `echo fcc00000.dwc3 > /sys/bus/platform/drivers/dwc3/bind`;
`echo otg > /sys/devices/platform/fe8a0000.usb2-phy/otg_mode`; `rmmod dwc3_gadget_otg`. Nothing persists across a reboot.

---

# Extra modules: tcp_bbr / sch_fq / usb_f_ncm / usb_f_ecm (built 2026-11-19)

STATUS: all four are **built and statically verified**; **none has been loaded on the device** (no adb was used).

| module | source (unmodified copy from the tree) | sha256 |
|---|---|---|
| `tcp_bbr.ko` | `net/ipv4/tcp_bbr.c` | `1e808e9b85036cde55949827dc04654f393d790eb946dfa4637fbdb020ad0a03` |
| `sch_fq.ko` | `net/sched/sch_fq.c` | `8f394fae0aaac343d7bacb40f25252f26f8a29320dea1ed0faea5f5306c7e077` |
| `usb_f_ncm.ko` | `drivers/usb/gadget/function/f_ncm.c` (+ `u_ether.h`, `u_ether_configfs.h`, `u_ncm.h`) | `a17d18772a44afd78c0b3d66a2e8765dfc7eeefe82503904ea153001e29e04cc` |
| `usb_f_ecm.ko` | `drivers/usb/gadget/function/f_ecm.c` (+ `u_ether.h`, `u_ether_configfs.h`, `u_ecm.h`) | `f15344a5be20afda1a6da76298ace7a135ec623a5f50fd03d3be0e1bb16aa13f` |

## Build
Same machine, tree, commit (`f39b590ea11d`, fetched by sha), compiler and `.config` as the dwc3 build; **no CONFIG option was
enabled for these modules** (the `.config` still only differs by the two dwc3 switches). Scripts, run in `/root/rk-kmod/`:
1. `prep-tree.sh` — fetch the commit, `device.config` -> `.config`, `olddefconfig modules_prepare`, copy the running kernel's `Module.symvers` into the tree.
2. `build-extra-on-server.sh` — copies the source files byte-for-byte (sha256 equal to the tree's) into `/root/rk-kmod/extra-mods/` with
   `obj-m := tcp_bbr.o sch_fq.o usb_f_ncm.o usb_f_ecm.o` (`usb_f_ncm-y := f_ncm.o`, `usb_f_ecm-y := f_ecm.o`) and runs
   `make CC=aarch64-linux-gnu-gcc -C kernel M=extra-mods modules`. No source edits were needed. `u_ether.c` is not copied:
   the modules import `gether_*` from vmlinux (built in via CONFIG_USB_F_RNDIS=y). Output: `build-extra.log`.
3. `layout-check.sh` + `closure.py` — struct-layout cross-check, output `layout-check.log`, `tree-crcs-extra.txt`.

## Checks (all passed)
| module | vermagic | undefined symbols, all exported by the running kernel | `__versions` entries equal to `Module.symvers` |
|---|---|---|---|
| tcp_bbr | `4.19.232 SMP preempt mod_unload modversions aarch64` | 10 / 10 | 11 / 11 |
| sch_fq | same | 33 / 33 | 34 / 34 |
| usb_f_ncm | same | 62 / 62 | 63 / 63 |
| usb_f_ecm | same | 44 / 44 | 45 / 45 |

(`__versions` has one more entry than the undefined list: `module_layout`.) Checked on the build machine and again on the Mac against
this directory's `Module.symvers`. No symbol was missing, so no workaround and no CRC patching was needed.
All 12 `gether_*` symbols used by ncm/ecm are EXPORT_SYMBOL_GPL in the running kernel.

Struct layout:
- `CONFIG_TCP_CONG_BBR`, `CONFIG_TCP_CONG_ADVANCED`, `CONFIG_NET_SCH_FQ`, `CONFIG_USB_F_NCM/ECM`, `CONFIG_USB_CONFIGFS_NCM/ECM` are referenced
  in the tree's `.c/.h/Makefile` files only as `obj-$(CONFIG_...)` Makefile switches (grep) — no `#ifdef` in any struct — and were not enabled anyway.
- 754 exported-symbol CRCs were recomputed with genksyms from the tree (33 objects: tcp core, inet_connection_sock, win_minmax, sch_api,
  sch_generic, gen_stats, skbuff, sock, dev, rtnetlink, nlattr, rbtree, u_ether, composite, functions, gadget configfs, udc core, configfs item,
  hrtimer, eth, `module_layout`): **754 / 754 equal the running kernel**. Per struct, number of matching CRCs whose type closure contains
  the full definition: `sock` 499, `tcp_sock` 2 (`tcp_slow_start`, `tcp_cong_avoid_ai`), `inet_connection_sock` 2, `tcp_congestion_ops` 207,
  `rate_sample` 207, `Qdisc` / `Qdisc_ops` 503, `sk_buff` 538, `net_device` 525, `usb_function` / `usb_function_instance` /
  `usb_configuration` / `usb_composite_dev` 29, `usb_gadget` 61, `usb_ep` / `usb_request` 78, `gether` 2 (`gether_connect`, `gether_disconnect`),
  `qdisc_watchdog` 4, `minmax` 3.

Not verified: runtime behaviour (never loaded); compiler is GCC 12.2 vs the kernel's Linaro GCC 6.3.1, as for the dwc3 module.

## Use (UNTESTED)
```
insmod tcp_bbr.ko && sysctl -w net.ipv4.tcp_congestion_control=bbr     # check net.ipv4.tcp_available_congestion_control
insmod sch_fq.ko  && tc qdisc replace dev wlan0 root fq                 # or sysctl net.core.default_qdisc=fq (new qdiscs only)
insmod usb_f_ncm.ko ; insmod usb_f_ecm.ko                               # then mkdir /config/usb_gadget/g1/functions/ncm.usb0 (or ecm.usb0)
```
The modules register function names `ncm` / `ecm` (alias `usbfunc:ncm`, `usbfunc:ecm`). `rmmod` only after the function instance directory is removed.
