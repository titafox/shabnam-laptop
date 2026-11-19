#!/system/bin/sh
# vx <command...>  — run a command inside the Void Linux tree (mounts on first use). Remove /data/void to undo everything.
R=/data/void
if ! grep -q " $R/proc " /proc/mounts; then
  mount -t proc proc $R/proc
  mount -t sysfs sys $R/sys
  mount -o bind /dev $R/dev
  mount -o bind /dev/pts $R/dev/pts 2>/dev/null
  mount -t tmpfs -o mode=1777 tmpfs $R/tmp
  mount -t tmpfs -o mode=755 tmpfs $R/run
  echo "nameserver 1.1.1.1" > $R/etc/resolv.conf
  echo "nameserver 223.5.5.5" >> $R/etc/resolv.conf
fi
exec /system/bin/chroot $R /usr/bin/env -i HOME=/root TERM=xterm-256color PATH=/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin XDG_RUNTIME_DIR=/run/user0 /bin/sh -c "$*"
