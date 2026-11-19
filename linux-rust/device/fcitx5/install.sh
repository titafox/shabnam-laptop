#!/bin/sh
# Run inside the Void tree from a directory holding this folder's files (and ../local.conf as ./local.conf).
# Puts Chinese fonts, locale and the fcitx5 Pinyin setup in place. Safe to run again.
set -e
xbps-install -y noto-fonts-cjk-sans dbus fcitx5 fcitx5-chinese-addons fcitx5-gtk+3 fcitx5-gtk4 gtk+3-demo || true
grep -q '^zh_CN.UTF-8' /etc/default/libc-locales || { sed -i 's/^#zh_CN.UTF-8 UTF-8/zh_CN.UTF-8 UTF-8/' /etc/default/libc-locales; xbps-reconfigure -f glibc-locales; }
cp local.conf /etc/fonts/local.conf && fc-cache -f
mkdir -p /root/.config/fcitx5/conf /usr/share/fcitx5/themes/shabnam
cp profile config /root/.config/fcitx5/
cp conf/*.conf /root/.config/fcitx5/conf/
cp /usr/share/fcitx5/themes/default-dark/*.png /usr/share/fcitx5/themes/shabnam/
cp theme-shabnam/theme.conf /usr/share/fcitx5/themes/shabnam/
cp shabnam-ime /usr/local/bin/shabnam-ime && chmod 755 /usr/local/bin/shabnam-ime
mkdir -p /usr/share/shabnam && cp ime-test.ui /usr/share/shabnam/ime-test.ui
cp shabnam-ime-test /usr/local/bin/shabnam-ime-test && chmod 755 /usr/local/bin/shabnam-ime-test
