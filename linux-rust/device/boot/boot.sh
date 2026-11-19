#!/system/bin/sh
# boot.sh — started by init as soon as /data is mounted (see void-desktop.rc).
# Keeps the Android interface from coming up, stops the Android services nothing here needs,
# and runs the Linux session (niri desktop; Win+Esc toggles the Mac second-screen mode).
# Android's files are untouched: `touch /data/void-boot/disabled` and reboot gives it back.
PATH=/system/bin:/vendor/bin:$PATH
B=/data/void-boot
LOG=$B/boot.log
[ -e $B/disabled ] && exit 0
# If the previous attempt never got to the end (hang, crash, forced power-off), boot Android.
if [ -e $B/trying ]; then echo "$(date) previous attempt did not finish; booting Android" >> $LOG; exit 0; fi
touch $B/trying; sync
echo "$(date) starting" > $LOG

# init is about to start the Android interface (zygote). Wait until it has, then stop it for
# good; stopping earlier would be undone when init reaches that step.
i=0; while [ $i -lt 60 ] && [ "$(getprop init.svc.zygote)" != running ]; do sleep 0.5; i=$((i+1)); done
stop
stop vendor.hwcomposer-2-1

# Android services that only serve the Android interface: stop them to free memory and CPU.
# Kept on purpose: device nodes, logging, storage keys, adb, the USB gadget, the Wi-Fi driver service.
stop_extras() {
  STOPPED=
  for s in $(getprop | sed -n 's/^\[init\.svc\.\([^]]*\)\]: \[running\]$/\1/p'); do
    case $s in
      *camera*|*media*|*drm*|*audio*|*bluetooth*|*neuralnetworks*|*gnss*|*sensors*|*vibrator*|*light*|*memtrack*|*power*|*thermal*|*cas*|*allocator*|*composer*|gpu|installd|storaged|statsd|incidentd|traced*|wificond|lmkd|tombstoned|gatekeeperd|credstore|iorapd|update_engine|profcollectd|heapprofd|mdnsd|netd|*rkaiq*|*oemlock*|*weaver*|*dumpstate*|*health*storage*|*rknn*|*rockit*|*omx*|*outputmanager*|*gralloc*|idmap2d|hidl_memory)
        stop $s; STOPPED="$STOPPED $s";;
    esac
  done
  echo "stopped:$STOPPED" >> $LOG
}
# Show the Shabnam start-up picture while the rest comes up (the session removes it).
/data/void.sh "mkdir -p /var/log/shabnam; chmod 700 /var/log/shabnam; (echo 0 > /sys/class/graphics/fb0/blank; shabnam-splash /usr/share/shabnam/splash.bgra >/var/log/shabnam/splash.log 2>&1 &)"

stop_extras

(
  # The USB network must be added only after adb has claimed the port, or adb never shows up
  # on the computer at the other end of the cable.
  i=0; while [ $i -lt 60 ] && [ "$(getprop sys.usb.ffs.ready)" != 1 ]; do sleep 0.5; i=$((i+1)); done
  sleep 3
  sh $B/usb-net.sh >> $LOG 2>&1
  # The Mac sometimes ignores the network port when it appears a moment after the debug port.
  # If a computer is attached but the port has no link after a while, reconnect once.
  sleep 12
  G=/config/usb_gadget/g1
  if [ "$(cat /sys/class/udc/*/state 2>/dev/null)" = configured ] && [ "$(cat /sys/class/net/usb0/carrier 2>/dev/null)" != 1 ]; then
    U=$(cat $G/UDC); echo > $G/UDC; sleep 2; echo "$U" > $G/UDC; echo "usb: reconnected (network port had no link)" >> $LOG
  fi
  # Services that had not started yet during the first pass.
  sleep 5
  stop_extras
) &

# init kills everything a service started as soon as the service's own process exits, so the
# session must run in the foreground here: this script stays alive for as long as the desktop does.
(sleep 30
 if [ -n "$(pidof niri-mali)$(pidof subscreen-rx)" ]; then echo "$(date) session running" >> $LOG; fi
 rm -f $B/trying; sync) &
/data/void.sh "/root/session.sh >/var/log/shabnam/session.log 2>&1"

# The desktop asked for power off or restart (quick panel): do it the clean way, through init.
case "$(cat /data/void/run/mode 2>/dev/null)" in
  poweroff) echo "$(date) power off requested" >> $LOG; rm -f $B/trying; sync; reboot -p; sleep 30 ;;
  reboot)   echo "$(date) restart requested" >> $LOG; rm -f $B/trying; sync; reboot; sleep 30 ;;
esac
# Only reached if the session ended (it gave up, or was killed): give the screen to Android.
echo "$(date) session ended; starting Android" >> $LOG
rm -f $B/trying; sync
start vendor.hwcomposer-2-1
start
