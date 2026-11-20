#!/usr/bin/python3
# Flicker test driver (runs as root inside the chroot). usage: inner.py NAME [NSHOTS] [QUIET_S]
# Reads extra Chromium flags from /var/log/shabnam/flicker/NAME/flags, opens ONE test window through the
# shabnam-browser wrapper with a throw-away profile, measures rAF fps + CPU, takes a screenshot burst, closes.
import json, os, signal, subprocess, sys, time, glob
name = sys.argv[1]; nshots = int(sys.argv[2]) if len(sys.argv) > 2 else 45
quiet = float(sys.argv[3]) if len(sys.argv) > 3 else 3.0
D = '/var/log/shabnam/flicker/' + name
PROF = '/home/shabnam/flicker-profile'
env = dict(LD_LIBRARY_PATH='/opt/mali/mali:/opt/mali:/opt/mpp2/usr/lib/aarch64-linux-gnu',
           XDG_RUNTIME_DIR='/run/user0', WAYLAND_DISPLAY='wayland-1', LANG='zh_CN.UTF-8', HOME='/root',
           PATH='/usr/local/bin:/usr/bin:/usr/sbin:/bin:/sbin')
socks = sorted(glob.glob('/run/user0/niri.*.sock'), key=os.path.getmtime)
env['NIRI_SOCKET'] = socks[-1]
flags = open(D + '/flags').read().strip()
extra_env = {}
if os.path.exists(D + '/env'):
    for l in open(D + '/env').read().split():
        k, v = l.split('=', 1); extra_env[k] = v
def niri(*a):
    return subprocess.run(['niri', 'msg'] + list(a), env=env, capture_output=True, text=True, timeout=10)
def windows():
    try: return json.loads(niri('--json', 'windows').stdout)
    except Exception: return []
def mine():  # pids in my session
    r = []
    for p in os.listdir('/proc'):
        if p.isdigit():
            try:
                f = open('/proc/%s/stat' % p).read().rsplit(')', 1)[1].split()
                if int(f[3]) == proc.pid: r.append((int(p), int(f[11]) + int(f[12])))
            except Exception: pass
    return r
def cpu():
    f = open('/proc/stat').readline().split()[1:8]; f = list(map(int, f))
    return sum(f), f[3] + f[4], sum(t for _, t in mine())
def niri_cpu():
    try:
        p = subprocess.run(['pidof', 'niri-mali'], capture_output=True, text=True).stdout.split()[0]
        f = open('/proc/%s/stat' % p).read().rsplit(')', 1)[1].split(); return int(f[11]) + int(f[12])
    except Exception: return 0
res = dict(name=name, flags=flags, env=extra_env)
t_launch = time.time()
e2 = dict(env); e2['SHABNAM_BROWSER_FLAGS'] = flags; e2.update(extra_env)
proc = subprocess.Popen(['/usr/local/bin/shabnam-browser', '--user-data-dir=' + PROF, '--new-window',
                         'file:///opt/shabnam-flicker/a.html'], env=e2, stdout=open(D + '/chromium.log', 'w'),
                        stderr=subprocess.STDOUT, start_new_session=True, cwd='/')
def title():
    for w in windows():
        if w.get('pid') == proc.pid: return w
    return None
try:
    w = None
    while time.time() - t_launch < 14:
        w = title()
        if w and (w.get('title') or '').startswith('F') and int(w['title'][1:].split()[0]) > 0: break
        time.sleep(0.4)
    res['window'] = w; res['startup_s'] = round(time.time() - t_launch, 1)
    if not w or not (w.get('title') or '').startswith('F'):
        res['error'] = 'page did not appear'; raise SystemExit
    time.sleep(1.5)
    def sample():
        for _ in range(5):
            try:
                w = title(); f, ms = w['title'][1:].split()[:2]; return int(f), int(ms), time.time()
            except Exception: time.sleep(0.1)
        raise SystemExit
    a = sample(); c0 = cpu(); n0 = niri_cpu()
    time.sleep(quiet)
    b = sample(); c1 = cpu(); n1 = niri_cpu()
    res['fps'] = round((b[0] - a[0]) / (b[2] - a[2]), 1)
    res['fps_pageclock'] = round((b[0] - a[0]) * 1000.0 / max(1, b[1] - a[1]), 1)
    tot = c1[0] - c0[0]
    res['cpu_total_pct_of_4cores'] = round(100.0 * (tot - (c1[1] - c0[1])) / tot, 1)
    res['cpu_chromium_pct_1core'] = round(100.0 * (c1[2] - c0[2]) / (tot / 4.0), 1)
    res['cpu_niri_pct_1core'] = round(100.0 * (n1 - n0) / (tot / 4.0), 1)
    for p, _ in mine():
        try:
            c = open('/proc/%d/cmdline' % p).read().replace('\0', ' ').split()
            if '--type=gpu-process' in c:
                res['gpu_args'] = [x for x in c if x.startswith(('--use-', '--ozone', '--render', '--enable-features', '--disable-features', '--gpu-pref')) and not x.startswith('--gpu-preferences')]
                m = open('/proc/%d/maps' % p).read()
                res['gpu_libs'] = sorted(set(l.split(None, 5)[5] for l in m.splitlines() if len(l.split(None, 5)) == 6 and any(k in l for k in ('mali', 'EGL', 'GLES', 'gbm', 'swiftshader', 'vulkan', 'vk_', 'dmabuf', '/dev/', 'memfd', 'shm'))))
                fds = {}
                for fd in os.listdir('/proc/%d/fd' % p):
                    try: t = os.readlink('/proc/%d/fd/%s' % (p, fd)); t = t.split(':[')[0]; fds[t] = fds.get(t, 0) + 1
                    except Exception: pass
                res['gpu_fds'] = {k: v for k, v in fds.items() if not k.startswith(('socket', 'pipe', '/usr/', '/home', 'anon_inode'))}
                res['gpu_threads'] = sorted(set(open('/proc/%d/task/%s/comm' % (p, t)).read().strip() for t in os.listdir('/proc/%d/task' % p)))
        except Exception as e: res['gpu_err'] = str(e)
    t_end = t_launch + 21.5; shots = 0; tb = time.time(); gap = float(os.environ.get('FL_GAP', '0.3'))
    for i in range(nshots):
        if time.time() > t_end: break
        t1 = time.time()
        niri('action', 'screenshot-screen', '-p', 'false', '--path', '%s/%03d.png' % (D, i))
        shots += 1
        time.sleep(max(0, gap - (time.time() - t1)))
    res['requested'] = shots; res['burst_s'] = round(time.time() - tb, 1)
finally:
    w = title()
    if w: niri('action', 'close-window', '--id', str(w['id']))
    for _ in range(15):
        if proc.poll() is not None: break
        time.sleep(0.2)
    if proc.poll() is None: proc.terminate(); time.sleep(1)
    for p, _ in mine():
        try: os.kill(p, signal.SIGKILL)
        except Exception: pass
    res['window_open_s'] = round(time.time() - t_launch, 1)
    time.sleep(0.3)
    for _ in range(40):
        got = len(glob.glob(D + '/*.png'))
        if got >= res.get('requested', 0): break
        time.sleep(0.25)
    time.sleep(0.5)
    res['shots'] = len(glob.glob(D + '/*.png'))
    res['leftover'] = [p for p, _ in mine()]
    json.dump(res, open(D + '/result.json', 'w'), indent=1)
    print(json.dumps(res))
