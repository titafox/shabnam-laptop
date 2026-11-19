# EC-NB01-CE 现状

日期：2026-11-18

## 目标

1. 用开放桌面从 Mac 控制这台安卓云电脑（打开页面、列应用、截图）。
2. 拿到 adb 或 root，以便改系统属性、停掉厂商限制、必要时换系统。
3. 日常用来开网页。若换系统，希望更轻，但不能把机器刷死。

## 已经确定的设备

| 项 | 值 |
|---|---|
| 型号 | EC-NB01-CE，升腾，RK3568 |
| 系统 | Android 11，`userdebug`，`release-keys` |
| 指纹 | `Centerm/rk3568_r/rk3568_r:11/RD2A.211001.002/eng.xiex.20241029.175252:userdebug/release-keys` |
| 内存 | 4GB |
| 地址 | `10.158.25.29`，走网线 |
| bootloader | 已解锁（`ro.boot.flash.locked=0`，verified boot 为 orange） |
| USB 调试 | 开着，`adbd` 在跑 |
| 网络 adb 端口 | `service.adb.tcp.port` 为空或 `0`，5555 不监听 |
| 无线调试开关 | `adb_wifi_enabled=0` |
| USB 设备控制器 | `sys.usb.controller` 为空，没有 gadget。USB-A 是 Host |
| 已装应用 | 约 181 个。没有 Magisk、KernelSU、SuperSU、KingRoot、Shizuku |

`userdebug` 只表示连上 adb 之后可以 `adb root`。普通应用不能写 `adbd`，也不能变成 root。

## 已经在用的控制通道

- Mac 控制服务：`lan-helper/server.py`，监听 `0.0.0.0:8000`。这台 Mac 是 `10.158.25.5`。
- 本机面板：http://127.0.0.1:8000/panel （只接受本机访问）。
- 设备上的开放桌面：`local.cloud.launcher`。常驻 `AgentService` 每 3 秒拉取 `http://10.158.25.5:8000/diagnostic-action.txt`，只执行用 `lan-helper/.install-keys/private.pem` 签过的一行。
- 下发命令：`lan-helper/ctl.py`，或面板上的按钮。回执在 `lan-helper/events.jsonl`。
- 现有命令：`inspect`、`list`、`activities`、`launch`、`start`、`open`、`url`、`home`、`usb-settings`、`appinfo`、`uninstall`、`install`、`screenshot`、`shotsetup`、`broadcast`。
- 截图需要设备上授权一次投屏。授权随服务重启丢失。最近一次截图回执是 `screenshot:no-permission`。
- 悬浮窗权限当时是开的。

## 已经看清的厂商软件

- 右侧悬浮框：升腾 `FloatView`，包名 `com.centerm.floatingWindow`。系统应用，控制通道卸不掉。应用信息页已经打开过，只能在设备上点停用或强行停止。强行停止后重启可能再出现。
- 桌面「设置」发的是通用设置动作，被上海移动的「设置应用」`com.shcmcc.setting` 接走。这不是安卓系统设置。
- 真正的系统设置是 `com.android.settings`，用组件名可以直接打开，已经成功拉起过。
- `com.shcmcc.setting` 里不用密码就能打开的只有导出的页面：`NetActivity`（网络）、`QuickAssistantActivity`（快捷助手）、`MainActivity`。纳管地址如果在设置里，就在网络页。
- 安全、诊断、升级、设备信息没有导出。外部拉起的回执是 `failed:start`。高级配置的密码框挡在这个应用内部。
- 没有查到 EC-NB01 这一页的默认密码。升腾老云教室网页后台的 `admin` / `admin` 不是这个对话框。
- 密码只有在安装包里用明文比较时，解包才能看到。该包在系统分区，没有 adb 就拉不出来。平台下发或只存哈希时，解包也得不到能输入的口令。

## 桌面改动

源码里「设置」已改为直接打开 `com.android.settings` / `com.android.settings.Settings`。包版本改为 7，文件是 `lan-helper/dist/desktop.apk`。

已安装的开放桌面证书是 `CN=LocalDesktop`。新包证书是 `CN=DeviceDiagnostics2`。原来的 LocalDesktop 私钥不在这台 Mac 上，不能覆盖安装。设备上已打开过下载页 `http://10.158.25.5:8000/desktop.apk`。需要先卸载现在的开放桌面，再安装这个包，并把它选成默认桌面。卸载后控制通道会断，所以要先把 apk 下到设备上。

## 打不开 adb 的原因

- Android 11 原生无线调试的 `Settings$WirelessDebuggingActivity` 不在 `com.android.settings` 里。开发者选项里的「无线调试」是空项。重装「设置入口」`local.helper.settings` 只是再去打开这个不存在的页面。
- 瑞芯微自己的网络 adb 用属性 `persist.internet_adb_enable=1`，监听 5555，网线即可，不需要 Wi-Fi。开发者选项里如果没有「网络 ADB」或「ADB over network」，就是菜单被拿掉了。普通应用写不了这个属性。
- 临时打开端口是 `setprop service.adb.tcp.port 5555`，然后 `stop adbd`、`start adbd`。重启后会掉。要保留，用 `persist.internet_adb_enable`。
- 常见调试端口 22、23、5555 等都没开。
- 开发者选项里的 Select DSU package 只能临时试一套通用系统镜像，不给当前系统 root，也不修 USB 设备口。这块升腾 vendor 很封闭，通用镜像经常黑屏或没有网络。

## 进刷机模式的尝试

关机后按住音量加再按电源，数据线接着。Mac 上 `system_profiler` 的 USB 树是空的，没有 Rockusb、Loader 或 Maskrom。机器随后也不再轮询控制通道。

没有查到 EC-NB01 的官方按键手册。同类 RK3568 云终端常见做法：

- Loader：关机，按住音量加，数据线接到电脑，再上电，约两秒后松开。电脑应出现 Rockusb。只有 USB-A Host 时，电脑认不到。
- Recovery：关机，牙签按住侧面 RECOVERY 小孔，再按一下电源。出现倒下的安卓机器人后松手。这个画面上再按电源，很多云终端会直接恢复出厂。
- Maskrom 不是按键，要短接 eMMC。还没试，也不该先试。

## root 和换系统

- 软件层没有可用的 root。Android 11 的一键 root 靠旧漏洞，对这套 2024-10 的 `release-keys` 无效。
- Magisk、KernelSU、APatch 都要改 boot 分区。bootloader 已解锁，但没有 Rockusb 就写不进去。
- 4GB 内存够几个普通网页标签。换系统的障碍不是内存，而是没有这块板子的现成 Armbian 设备树。开发板镜像对不上显示和网口。刷坏后如果没有 Rockusb 或调试串口，退不回来。

## 还没走通、也还没排除的路

1. 调试串口。RK3568 常见 1500000 8N1。有 shell 之后设置 `persist.internet_adb_enable=1`。
2. 找到真正的 USB 设备口或 OTG，而不是 USB-A Host。换一根能传数据的线，再按 Loader 步骤，同时看 Mac 的 USB 树。
3. 从开发者选项打开「高级重启」，用电源菜单进 Recovery。停在机器人画面，不要再按电源。Recovery 里如果出现 USB adb，再设网络 adb 属性。
4. 安装版本 7 的开放桌面，让桌面「设置」进系统设置。这不提供 root。

## ROOT 已到手（2026-11-19 更新）

**结论：不需要内核漏洞。** 设备是 `userdebug` + **SELinux Permissive** 构建：普通应用 uid 就能 `setprop`（含 `ctl.restart`）。`ro.adb.secure` 空 + `ro.debuggable=1` → `adb root` 直接给 root（`u:r:su:s0`）。内核 4.19.232，与漏洞无关。

**标准操作**（已验证跨重启）：
```
adb connect 10.158.25.29:5555     # 开机自动监听
adb root                          # uid=0
```

**持久化**：`persist.internet_adb_enable=1` 已设置（`/vendor/etc/init/hw/init.rk30board.rc` 触发 5555+restart adbd；落盘 `/data/property/persistent_properties`，仅恢复出厂清空）。坑：`com.centerm.androidmanager`（8000 端口 SOAP MDM）开机清零端口 → 已随厂商包一起卸载，重启后稳定。

**厂商清理**：16 个包 per-user 卸载（centerm×6、komect.centerm、sunniwell×2、shcmcc×3、chinamobile.middleware×2、cmcc.softdetector）。保留 cloudcomputer（串流客户端）与 komect.ota/hjqbind 等。默认输入法 AOSP Latin。

**通道增强**：AgentService 新增 `probe <sh命令>`（cros-5，versionCode 104），`ctl.py probe 'cmd'`，stdout 在 events.jsonl。
