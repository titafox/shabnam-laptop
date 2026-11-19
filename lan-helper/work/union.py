import sys
p=sys.argv[1]; out=[]; seen=set(); inbase=False
for l in open(p,encoding='utf-8'):
    if l.startswith('|||||||'): inbase=True; continue
    if l.startswith('======='): inbase=False; continue
    if l.startswith('<<<<<<<') or l.startswith('>>>>>>>'): continue
    if inbase: continue
    if l.startswith('<uses-permission'):
        if l in seen: continue
        seen.add(l)
    out.append(l)
open(p,'w',encoding='utf-8').write(''.join(out))
