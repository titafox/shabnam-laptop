#!/usr/bin/env python3
# For each exported symbol in the given .symtypes files: does its genksyms type closure contain the FULL definition
# of the structs of interest?  Joined with tree CRC (from nm __crc_*) and the running kernel's Module.symvers.
import sys, re, subprocess, collections
symvers = {}
for l in open(sys.argv[1]):
    p = l.rstrip('\n').split('\t'); symvers[p[1]] = int(p[0], 16)
WANT = ['s#sock','s#tcp_sock','s#inet_connection_sock','s#tcp_congestion_ops','s#rate_sample','s#Qdisc','s#Qdisc_ops','s#sk_buff',
        's#net_device','s#usb_function','s#usb_function_instance','s#usb_configuration','s#usb_composite_dev','s#usb_gadget',
        's#usb_ep','s#usb_request','s#gether','s#config_group','s#hrtimer','s#qdisc_watchdog','s#minmax']
tot = collections.Counter(); ok = collections.Counter(); allsyms = 0; allok = 0; bad = []
out = open(sys.argv[2], 'w')
for st in sys.argv[3:]:
    defs = {}; exports = []
    for l in open(st):
        t = l.split()
        if not t: continue
        defs[t[0]] = t[1:]
        if not re.match(r'^[a-zA-Z]#', t[0]): exports.append(t[0])
    crcs = {}
    for l in subprocess.run(['aarch64-linux-gnu-nm', st[:-len('.symtypes')] + '.o'], capture_output=True, text=True).stdout.splitlines():
        p = l.split()
        if len(p) == 3 and p[2].startswith('__crc_'): crcs[p[2][6:]] = int(p[0], 16)
    for e in exports:
        if e not in crcs: continue
        seen = set(); stack = [e]
        while stack:
            for tok in defs.get(stack.pop(), []):
                if re.match(r'^[a-zA-Z]#', tok) and tok not in seen: seen.add(tok); stack.append(tok)
        full = [w for w in WANT if w in seen and 'UNKNOWN' not in defs.get(w, ['UNKNOWN'])]
        run = symvers.get(e); allsyms += 1
        m = (run == crcs[e]); allok += m
        if not m: bad.append((e, crcs[e], run, st))
        for w in full: tot[w] += 1; ok[w] += m
        out.write('0x%08x\t%s\t%s\t%s\n' % (crcs[e], e, 'match' if m else ('MISMATCH' if run is not None else 'not-exported-by-running-kernel'), ','.join(x[2:] for x in full)))
print('exported symbols recomputed: %d, equal to running kernel: %d' % (allsyms, allok))
for e, c, r, st in bad: print('  DIFF %s tree=0x%08x running=%s (%s)' % (e, c, 'none' if r is None else '0x%08x' % r, st))
for w in WANT: print('  %-26s full definition covered by %d symbol CRCs, %d match' % (w[2:], tot[w], ok[w]))
