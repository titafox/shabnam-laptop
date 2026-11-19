#!/usr/bin/env python3
"""向设备控制通道下发一条已签名命令，并读回设备的回执。

用法:
  ctl.py inspect
  ctl.py list                         # 列出设备全部应用（回执里带 packages）
  ctl.py launch <pkg>
  ctl.py uninstall <pkg>              # 设备上弹卸载确认
  ctl.py appinfo <pkg>               # 打开应用信息（停用/强停/清数据）
  ctl.py open <intent-action>        # 如 android.settings.USB_SETTINGS
  ctl.py start <pkg/类名>            # 如 com.android.settings/.Settings$WirelessDebuggingActivity
  ctl.py url <https://...>
  ctl.py usb-settings | home
  ctl.py install <apk路径>           # 等价于 queue-apk.py
"""
import base64, hashlib, json, os, subprocess, sys, time, uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parent
DIST = ROOT / "dist"
KEY = ROOT / ".install-keys/private.pem"
ACTION = DIST / "diagnostic-action.txt"
EVENTS = ROOT / "events.jsonl"


def sign_and_queue(command: str) -> str:
    req_id = uuid.uuid4().hex
    payload = f"{req_id} {command}".strip()
    sig = subprocess.check_output(
        ["openssl", "dgst", "-sha256", "-sign", str(KEY)], input=payload.encode())
    line = payload + " " + base64.b64encode(sig).decode() + "\n"
    tmp = DIST / ".action.tmp"
    tmp.write_text(line)
    os.replace(tmp, ACTION)
    return payload


def stage_apk(path: str) -> str:
    src = Path(path)
    size = src.stat().st_size
    if not 0 < size <= 256 * 1024 ** 2:
        sys.exit("APK 必须 >0 且 <=256MiB")
    digest = hashlib.sha256()
    with src.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            digest.update(chunk)
    name = digest.hexdigest()
    dest = DIST / (name + ".apk")
    if not dest.exists():
        import shutil
        shutil.copyfile(src, dest)
    return name


def wait_response(payload: str, timeout: float = 25.0):
    """等待设备回传 request == payload 的那条 /log 记录。"""
    start = time.time()
    seen = EVENTS.stat().st_size if EVENTS.exists() else 0
    while time.time() - start < timeout:
        if EVENTS.exists():
            with EVENTS.open("rb") as f:
                f.seek(seen)
                data = f.read()
                seen += len(data)
            for raw in data.splitlines():
                try:
                    ev = json.loads(raw)["event"]
                except Exception:
                    continue
                if ev.get("request") == payload:
                    return ev
        time.sleep(1)
    return None


def main():
    if len(sys.argv) < 2:
        print(__doc__); sys.exit(1)
    verb = sys.argv[1]
    if verb == "install":
        name = stage_apk(sys.argv[2])
        command = f"install {name}"
    else:
        command = " ".join([verb] + sys.argv[2:])
    shot = ROOT / "last-shot.png"
    shot_before = shot.stat().st_mtime if shot.exists() else 0
    payload = sign_and_queue(command)
    print(f"→ 已下发: {payload}")
    if verb == "screenshot":
        import time as _t
        for _ in range(25):
            if shot.exists() and shot.stat().st_mtime > shot_before:
                print(f"← 截图已保存: {shot}")
                return
            _t.sleep(1)
        print("（未收到截图——可能未授权投屏，先执行 shotsetup 并在设备上点『立即开始』）")
        return
    ev = wait_response(payload)
    if ev is None:
        print("（未在超时内收到回执——设备可能不在桌面前台，或正忙于下载安装）")
        return
    print(f"← 回执 exec={ev.get('exec')}")
    acts = ev.get("activities")
    if acts is not None:
        print(f"  {len(acts)} 个 Activity:")
        for a in acts:
            if "error" in a:
                print(f"   ! {a['error']}"); continue
            ex = "导出" if a.get("exported") else "未导出"
            en = "" if a.get("enabled", True) else " [停用]"
            print(f"   [{ex}]{en} {a.get('name')}")
    pkgs = ev.get("packages")
    if pkgs is not None:
        pkgs.sort(key=lambda p: (not p.get("system"), p.get("pkg", "")))
        print(f"  共 {len(pkgs)} 个应用:")
        for p in pkgs:
            tag = "系统" if p.get("system") else "用户"
            en = "" if p.get("enabled", True) else " [已停用]"
            lf = "" if p.get("launchable") else " (无图标)"
            print(f"   [{tag}] {p.get('pkg')}  «{p.get('label')}» v{p.get('version')}{en}{lf}")


if __name__ == "__main__":
    main()
