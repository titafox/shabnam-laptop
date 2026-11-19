#!/system/bin/sh
# usb-net.sh — run as root on the laptop (Android side, not inside the Void tree).
# Adds a USB network function (NCM, which macOS supports natively) next to adb on port A,
# so the Mac gets a direct network link over the Type-C cable. Verified 2026-11-20:
# 311 Mbit/s Mac -> laptop, ~1.2 ms round trip, on a USB 2.0 (480 Mbit/s) link.
# Needs usb_f_ncm.ko loaded (it is loaded at boot from /vendor/lib/modules).
G=/config/usb_gadget/g1
# How this machine introduces itself to the computer it is plugged into.
MAKER=Shabnam
PRODUCT=Laptop
[ -d $G/functions/ncm.usb0 ] || mkdir $G/functions/ncm.usb0
if [ ! -e $G/configs/b.1/f2 ] || [ "$(cat $G/strings/0x409/manufacturer)" != "$MAKER" ] || [ "$(cat $G/strings/0x409/product)" != "$PRODUCT" ]; then
  echo none > $G/UDC                      # no pause here: Android's init re-binds within a second
  echo "$MAKER" > $G/strings/0x409/manufacturer
  echo "$PRODUCT" > $G/strings/0x409/product
  [ -e $G/configs/b.1/f2 ] || ln -s $G/functions/ncm.usb0 $G/configs/b.1/f2
  echo fcc00000.dwc3 > $G/UDC
  sleep 3
fi
ip link set usb0 up
# The Mac assigns itself a 169.254.x.y address when no DHCP server answers; join that range.
ip addr replace 169.254.1.1/16 dev usb0
ip addr replace 10.55.0.1/24 dev usb0   # the range our DHCP service hands to the Mac
# Android routes only through per-network tables; send this range through the main table.
ip rule add pref 5000 to 10.55.0.0/24 lookup main 2>/dev/null
ip rule add pref 5003 to 169.254.0.0/16 lookup main 2>/dev/null
ip -br addr show usb0
