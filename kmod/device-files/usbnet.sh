#!/vendor/bin/sh
# usbnet.sh on|off|status  — add/remove a USB network card (NCM) next to adb on Type-C port0.
# With "on" a Mac/PC sees a new Ethernet interface; this side is 169.254.77.1 (link-local,
# the computer self-assigns an address, no setup needed). ~40 MB/s. Not persistent across
# reboot or across a USB mode change made by Android.
PATH=/vendor/bin:/system/bin:$PATH
G=/config/usb_gadget/g1
C=$G/configs/b.1
case "$1" in
on)
  mkdir -p $G/functions/ncm.gs9 || { echo "ncm function unavailable"; exit 1; }
  echo none > $G/UDC
  rm -f $C/f1 $C/f2 $C/f3 $C/f4
  ln -s $G/functions/ffs.adb $C/f1
  ln -s $G/functions/ncm.gs9 $C/f2
  echo fcc00000.dwc3 > $G/UDC
  i=0; while [ $i -lt 10 ] && ! ip link show usb0 >/dev/null 2>&1; do sleep 0.5; i=$((i+1)); done
  ifconfig usb0 169.254.77.1 netmask 255.255.0.0 up
  ip route replace 169.254.0.0/16 dev usb0 table local_network
  echo "on: usb0 169.254.77.1" ;;
off)
  svc usb setFunctions
  echo off ;;
*)
  ip addr show usb0 2>/dev/null | grep -E "state|inet " || echo "usb network is off" ;;
esac
