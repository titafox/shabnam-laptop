#!/system/bin/sh
# Runs on the laptop's Android side as root (pushed there by run-test.sh):  device-run.sh NAME "FLAGS" [SHOTS] [ENV]
# Installs the pages to /opt/shabnam-flicker, opens ONE test browser window for about 23 s, leaves
# /data/local/tmp/fl-NAME.tar (screenshots + result.json + chromium.log).
NAME=$1; FLAGS=$2; N=${3:-45}; ENVS=$4
V=/data/void; P=$V/opt/shabnam-flicker; D=$V/var/log/shabnam/flicker/$NAME
mkdir -p $P && for f in a.html b.html inner.py; do cp /data/local/tmp/fl-$f $P/$f; done
chown -R 0:0 $P; chmod 755 $P; chmod 644 $P/*
rm -rf $D; mkdir -p $D; chmod 700 $V/var/log/shabnam/flicker
echo "$FLAGS" > $D/flags
[ -n "$ENVS" ] && echo "$ENVS" > $D/env
/data/void.sh "python3 /opt/shabnam-flicker/inner.py $NAME $N"
cd $D && tar cf /data/local/tmp/fl-$NAME.tar . && chmod 644 /data/local/tmp/fl-$NAME.tar
rm -rf $V/var/log/shabnam/flicker $V/home/shabnam/flicker-profile
