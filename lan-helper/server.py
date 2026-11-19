import json
from datetime import datetime, timezone
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from threading import Lock
from urllib.parse import urlparse

import ctl

ROOT = Path(__file__).resolve().parent
LOCK = Lock()
PANEL = ROOT / "panel.html"
VERBS = {
    "inspect", "list", "screenshot", "shotsetup", "home", "usb-settings",
    "launch", "url", "open", "start", "appinfo", "uninstall", "activities",
}

class Handler(SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def local_only(self):
        return self.client_address[0] in ("127.0.0.1", "::1")

    def send_json(self, code, obj):
        body = json.dumps(obj, ensure_ascii=False).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        path = urlparse(self.path).path
        if path in ("/panel", "/panel/"):
            if not self.local_only():
                self.send_error(403)
                return
            data = PANEL.read_bytes()
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            return
        if path == "/api/state":
            if not self.local_only():
                self.send_error(403)
                return
            self.send_json(200, latest_state())
            return
        if path == "/api/shot":
            if not self.local_only():
                self.send_error(403)
                return
            shot = ROOT / "last-shot.png"
            if not shot.exists():
                self.send_error(404)
                return
            data = shot.read_bytes()
            self.send_response(200)
            self.send_header("Content-Type", "image/png")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            return
        super().do_GET()

    def do_POST(self):
        path = urlparse(self.path).path
        if path == "/api/cmd":
            if not self.local_only():
                self.send_error(403)
                return
            try:
                size = int(self.headers.get("Content-Length", "0"))
                if not 0 < size <= 4096:
                    self.send_error(413)
                    return
                body = json.loads(self.rfile.read(size))
                command = " ".join(str(body.get("command", "")).split())
            except (ValueError, UnicodeError):
                self.send_error(400)
                return
            verb = command.split(" ", 1)[0] if command else ""
            if verb not in VERBS or any(c in command for c in "\n\r"):
                self.send_json(400, {"error": "不支持的指令"})
                return
            payload = ctl.sign_and_queue(command)
            self.send_json(200, {"queued": payload})
            return
        if self.path == '/shot':
            size = int(self.headers.get('Content-Length', '0'))
            if not 0 < size <= 32 * 1024 * 1024:
                self.send_error(413)
                return
            data = self.rfile.read(size)
            ts = datetime.now(timezone.utc).strftime('%Y%m%d-%H%M%S')
            with LOCK:
                (ROOT / 'last-shot.png').write_bytes(data)
                shots = ROOT / 'shots'
                shots.mkdir(exist_ok=True)
                (shots / f'{ts}.png').write_bytes(data)
            self.send_response(204)
            self.end_headers()
            return
        if self.path != '/log':
            self.send_error(404)
            return
        try:
            size = int(self.headers.get('Content-Length', '0'))
            if not 0 < size <= 4 * 1024 * 1024:
                self.send_error(413)
                return
            event = json.loads(self.rfile.read(size))
            entry = {'received': datetime.now(timezone.utc).isoformat(),
                     'client': self.client_address[0], 'event': event}
            with LOCK:
                with (ROOT / 'events.jsonl').open('a', encoding='utf-8') as log:
                    log.write(json.dumps(entry, ensure_ascii=False) + '\n')
            self.send_response(204)
            self.end_headers()
        except (ValueError, UnicodeError):
            self.send_error(400)

def latest_state():
    events = ROOT / "events.jsonl"
    entry = None
    if events.exists() and events.stat().st_size:
        with events.open("rb") as f:
            f.seek(max(0, events.stat().st_size - 2_000_000))
            raw = f.read().splitlines()
        for line in reversed(raw):
            try:
                entry = json.loads(line)
                break
            except ValueError:
                continue
    shot = ROOT / "last-shot.png"
    now = datetime.now(timezone.utc)
    received = entry.get("received") if entry else None
    age = None
    if received:
        try:
            age = (now - datetime.fromisoformat(received)).total_seconds()
        except ValueError:
            age = None
    event = (entry or {}).get("event") or {}
    return {
        "online": age is not None and age < 12,
        "age": age,
        "received": received,
        "client": (entry or {}).get("client"),
        "exec": event.get("exec"),
        "model": event.get("model"),
        "android": event.get("android"),
        "overlay": event.get("overlay"),
        "projection": event.get("projection"),
        "adb_enabled": event.get("adb_enabled"),
        "adb_wifi_enabled": event.get("adb_wifi_enabled"),
        "properties": event.get("properties") or {},
        "packages": event.get("packages"),
        "activities": event.get("activities"),
        "shot": shot.exists(),
        "shot_mtime": shot.stat().st_mtime if shot.exists() else 0,
    }


if __name__ == '__main__':
    server = ThreadingHTTPServer(('0.0.0.0', 8000), partial(Handler, directory=str(ROOT / 'dist')))
    print('安卓设置助手已启动，端口 8000；日志：' + str(ROOT / 'events.jsonl'), flush=True)
    server.serve_forever()
