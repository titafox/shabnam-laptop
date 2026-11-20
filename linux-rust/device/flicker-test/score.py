#!/usr/bin/env python3
# score.py RUN_DIR [RUN_DIR...]  — counts screenshots whose page area violates the test page's invariants:
#  (1) every pixel row is one colour across the width; (2) stripe edges are 60 device px apart.
import sys, glob, os, json
import numpy as np
from PIL import Image
X0, X1, Y0, Y1 = 32, 1888, 164, 970     # page area inside the 1920x1080 screenshot (full-width window, scale 1.5)
TH = 14; STRIPE = 60
def score(path):
    a = np.asarray(Image.open(path).convert('RGB'), dtype=np.int16)
    if a.shape[:2] != (1080, 1920): return 'size', 0
    r = a[Y0:Y1, X0:X1]
    spread = (r.max(axis=1) - r.min(axis=1)).max(axis=1)      # per row
    bad = int((spread > TH).sum())
    if bad: return 'rows', bad
    row = np.median(r, axis=1)
    d = np.abs(np.diff(row, axis=0)).max(axis=1)
    idx = np.nonzero(d > 10)[0]
    if len(idx) == 0: return 'blank', 0
    edges = []; grp = [idx[0]]
    for i in idx[1:]:
        if i - grp[-1] <= 3: grp.append(i)
        else: edges.append(sum(grp) / len(grp)); grp = [i]
    edges.append(sum(grp) / len(grp))
    gaps = np.diff(edges)
    off = int((np.abs(gaps - STRIPE) > 2.5).sum())
    if off: return 'spacing', off
    return 'ok', 0
tot = {}
for d in sys.argv[1:]:
    res = {}
    for p in sorted(glob.glob(d + '/*.png')):
        k, n = score(p); res.setdefault(k, []).append((os.path.basename(p), n))
    n = sum(len(v) for v in res.values())
    g = len(res.get('rows', [])) + len(res.get('spacing', []))
    print('%s: glitched %d/%d  (rows %d, spacing %d, blank %d, ok %d)' % (os.path.basename(d.rstrip('/')), g, n,
          len(res.get('rows', [])), len(res.get('spacing', [])), len(res.get('blank', [])), len(res.get('ok', []))))
    for k in ('rows', 'spacing', 'size'):
        if k in res: print('   ', k, ' '.join('%s:%d' % x for x in res[k][:12]))
