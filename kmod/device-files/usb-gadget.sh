#!/vendor/bin/sh
# Make Type-C port0 a USB peripheral (adb / file transfer to a computer). Added 2026-11-19.
# The stock kernel builds dwc3 host-only; this loads a dwc3 build with gadget support and
# rebinds the OTG controller to it. To disable: delete /vendor/etc/init/usb-gadget.rc
PATH=/vendor/bin:/system/bin:$PATH
M=/vendor/lib/modules/dwc3_gadget_otg.ko
D=/sys/devices/platform/usbdrd/fcc00000.dwc3
P=/sys/devices/platform/fe8a0000.usb2-phy/otg_mode
S=/sys/class/udc/fcc00000.dwc3/state
T=/sys/class/typec/port0/port_type
MARK=/metadata/usb-gadget.trying
LOG=/metadata/usb-gadget.status

# if the previous attempt never finished (crash/reboot while loading), do not try again
if [ -e $MARK ]; then echo skipped > $LOG; exit 0; fi
echo 1 > $MARK; sync

bind_gadget() {
  echo dwc3-gadget-otg > $D/driver_override
  echo fcc00000.dwc3 > /sys/bus/platform/drivers/dwc3-gadget-otg/bind
  echo peripheral > $P
}

i=0
while [ ! -e $D/driver ] && [ $i -lt 20 ]; do sleep 0.5; i=$((i+1)); done
[ -d /sys/bus/platform/drivers/dwc3-gadget-otg ] || insmod $M
# extra gadget functions (network card for a Mac); harmless if they fail to load
for m in usb_f_ncm usb_f_ecm; do
  [ -f /vendor/lib/modules/$m.ko ] && insmod /vendor/lib/modules/$m.ko 2>/dev/null
done
cur=$(basename "$(readlink $D/driver)")
if [ "$cur" != dwc3-gadget-otg ]; then
  echo fcc00000.dwc3 > /sys/bus/platform/drivers/dwc3/unbind
  bind_gadget
fi
# Never let the other side turn this port into a host (a Mac is dual-role too and
# sometimes wins the negotiation, which silently kills the gadget).
[ -e $T ] && echo sink > $T
echo peripheral > $P
setprop sys.usb.controller fcc00000.dwc3
if [ -d /sys/class/udc/fcc00000.dwc3 ]; then echo ok > $LOG; else echo no-udc > $LOG; fi
rm -f $MARK; sync

# Watchdog. Two failure shapes seen on this hardware:
#  - after the system re-plugs the gadget the host keeps resetting it and the state stays
#    "default": cycling the phy mode recovers it;
#  - if that does not help, rebinding the controller driver does.
n=0; tries=0
while true; do
  sleep 2
  [ -e $S ] || { sleep 5; [ -e $S ] || { bind_gadget; echo "rebound(no-udc) $(date)" >> $LOG; sleep 5; }; continue; }
  if [ "$(cat $S)" = default ]; then n=$((n+1)); else n=0; tries=0; fi
  if [ $n -ge 2 ]; then
    if [ $tries -lt 2 ]; then
      echo otg > $P; sleep 1; echo peripheral > $P
      echo "phy-cycled $(date)" >> $LOG
    else
      echo fcc00000.dwc3 > /sys/bus/platform/drivers/dwc3-gadget-otg/unbind; sleep 1
      bind_gadget
      echo "rebound $(date)" >> $LOG
      tries=0
    fi
    tries=$((tries+1)); n=0; sleep 4
  fi
done
