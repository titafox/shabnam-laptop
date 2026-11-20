# 教程：锁死的移动云电脑，不拆机拿到 root（以及之后的桌面/副屏）

适用：中国移动云电脑 **EC-NB01-CE**（RK3568，Android 11 `userdebug` 构建，
`ro.adb.secure` 为空，SELinux Permissive——同族机器先按第 2 步的条件核对）。
全程不拆机；人只需要操作机器约 5 分钟，其余都在 Mac 终端里做。
仓库：`loptop`（本仓库），全部现成产物都在里面。

## 第一部分：突破——开出网络 adb，拿到 root

「锁死」的问题到这一步就**彻底解决了**。下面的桌面/副屏都是在 root 之下的
普通操作，第一部分独立成立。

### 第 1 步：手动三步（全程唯一必须人操作的）

1. 按 **`Win+N`**，下拉状态栏 → **用户 → 更多设置**：这是安卓原生设置
   （出厂的「设置」被 `com.shcmcc.setting` 劫持，从这里进可以绕开）。
2. 原生设置 → **关于** → 连点 **7 次版本号**，唤出开发者选项，打开 **USB ADB**
   （此时 USB 设备模式被固件禁着，口不通，先开着即可，真正用上是网络 adb）。
3. 原生设置 → **更多应用** → **闪电（Lightning）浏览器** → 打开
   `http://10.158.25.5:8000/desktop.apk` 安装「开放桌面」，设为默认桌面。

Mac 侧安装通道（开放桌面靠它下发命令）：

```sh
sudo ifconfig en0 alias 10.158.25.5     # Mac 地址固定（设备侧写死了）
cd loptop/lan-helper   # 提交 3b5698a 之前的版本
python3 server.py &                  # 8000 端口，desktop.apk 放 dist/
```

闪电浏览器自己不能执行任何命令，它只是"能装 APK"；开端口要靠装进来的 app
（本教程用开放桌面，换成 Shizuku/KeepADB 等任意能跑 shell 的 app 也行——原理
相同：app 权限 setprop）。

### 第 2 步：app 开端口 → adb root（核心突破，2 分钟）

**动手前先核对三个前提**（不满足就没有这条最短路径）。此时 5555 还没开，
核对走开放桌面的 `probe` 通道（Mac 端 `ctl.py probe '<命令>'`）：

```sh
getprop ro.build.type     # userdebug
getprop ro.adb.secure     # 必须为空
getprop ro.debuggable     # 1
getenforce                # Permissive
```

开放桌面的轮询通道下发一条 `probe` 命令（Mac 端 `ctl.py probe`）：

```sh
setprop service.adb.tcp.port 5555; setprop ctl.restart adbd
```

然后 Mac 上：

```sh
adb connect 10.158.25.29:5555    # 出厂 IP；换网/换机以实际 IP 为准
adb root                          # 直接 uid=0（u:r:su:s0）
adb shell id                      # 验证：uid=0(root)
```

**root 到手。** 零内核漏洞，没有 Magisk，没有刷任何东西。

### 第 3 步：固化，让重启不掉

```sh
# 网络 adb 跨重启（触发器本来就在 /vendor/etc/init/hw/init.rk30board.rc：
# on property:persist.internet_adb_enable=1 → setprop service.adb.tcp.port 5555 + restart adbd）
adb shell setprop persist.internet_adb_enable 1

# 开机清零 5555 的是 MDM，卸掉
adb shell pm uninstall -k --user 0 com.centerm.androidmanager
# （其余厂商包见仓库记录；root 在手后也可以直接物理删除）

adb reboot    # 重启后 adb connect 仍秒连、adb root 仍 uid=0 → 完成
```

第一部分到此结束：**任意一台同配置机器，人工 5 分钟 + 终端 5 分钟 = 稳定 root +
网络 adb，永不掉（仅恢复出厂清空）。**

---

## 第二部分：root 到手之后（按需做，都是普通操作）

顺序即依赖关系。

### ① 备份 + 救砖通道实测（动任何分区前的硬性前提）

这台机器没有进 Loader 的硬件键，U-Boot fastboot 是唯一救砖通道：

```sh
# Type-C port0（充电口侧）接 Mac
adb pull /dev/block/by-name/super  super.img    # 最大，走 USB 最快
adb pull /dev/block/by-name/boot   boot.img
adb pull /dev/block/by-name/vendor vendor.img   # 后面要改 vendor
# pull 前后各算一次 sha256，必须一致

adb reboot bootloader
fastboot devices                # 必须有设备
fastboot flash boot boot.img    # 写回验证通道
fastboot reboot                 # 偶尔要发两次
```

两条铁律：① 永远不用 `fastboot getvar all`（会卡死机器）；② `reboot bootloader`
之后先确认 `fastboot devices` 在线，再做不可逆操作。

### ② USB 外设模式（推荐；高速传输和副屏都靠它）

原厂内核 dwc3 只有 host。现成模块在 `kmod/`（对本机内核自编，CRC 核对过）：

```sh
adb push kmod/dwc3_gadget_otg.ko kmod/usb_f_ncm.ko kmod/usb_f_ecm.ko /vendor/lib/modules/
adb push kmod/device-files/usb-gadget.sh /vendor/bin/
adb push kmod/device-files/usb-gadget.rc /vendor/etc/init/
adb reboot
```

之后 port0 接 Mac 出现 USB adb（33 MB/s，Wi-Fi 只有 1.5）。**线必须插 port0**，
另一个口只能当主机。

### ③ Linux 桌面（目标一）


Void rootfs 放 `/data/void` 当 chroot，安卓只负责带内核，**没有安装这步**：

```sh
adb push linux-rust/device/void.sh                 /data/void.sh
adb push linux-rust/device/boot/boot.sh            /data/void-boot/boot.sh
adb push linux-rust/device/usb-net.sh              /data/void-boot/usb-net.sh
adb push linux-rust/device/boot/void-desktop.rc    /vendor/etc/init/
chmod 755 /data/void.sh /data/void-boot/*.sh
adb reboot
```

`void-desktop.rc` 在 `post-fs-data` 起 `boot.sh`：等 zygote 起来后 `stop` 安卓
界面 → 配 USB 网络 → 前台跑 `/root/session.sh`（niri 桌面，Win+Esc 切副屏）。

安全网：`touch /data/void-boot/disabled` 重启即回安卓；上次没启动成功下次自动回。

### ④ Mac 副屏（目标二）

USB NCM 网口（② 的模块 + `usb-net.sh`）+ Mac 端应用：


```sh
cd loptop/linux-rust/mac/ShabnamDisplay && make app
# 线接 port0，Mac 自动出现新网卡；笔记本按 Win+Esc
```

线缆走 10.55.0.x 段（dnsmasq），双向认证，H.264 硬编/硬解，键鼠反向控制 Mac；
声音三模式在 Mac 设置窗口选。

### 回退路径

| 改了什么 | 退回 |
|---|---|
| Linux 桌面 | `touch /data/void-boot/disabled` 重启 |
| vendor 改动 | `fastboot flash vendor vendor.img` |
| 一切 | uboot 没动过 → fastboot 通道一直在 → 可救 |

唯一不可逆风险是 boot/uboot 分区，全程没动。

### 依赖清单（本仓库之外）

| 项 | 获取 |
|---|---|
| Void aarch64 rootfs、打 Mali 补丁的 niri、厂商 Mali/MPP 库 | 从已装好的机器 `adb pull`（root 在手后是一句话的事） |
| 开放桌面 APK（第 1 步用） | **最新版 = 一体化桌面 cros-8（versionCode 107）**，源码在 git：`git checkout 3b5698a~1 lan-helper/desktop-cros`（提交 `3b5698a` 删除了它），`PLATFORM=0 ./build.sh` 出 `build/open-desktop.apk` |
