#!/usr/bin/env python3
# Extract exported symbol CRCs from an uncompressed arm64 Image (4.19, PREL32 ksymtab) -> Module.symvers
import struct, sys
boot, kallsyms, out = sys.argv[1:4]
IMG_OFF, IMG_SIZE = 2048, 31817736
img = open(boot, 'rb').read()[IMG_OFF:IMG_OFF + IMG_SIZE]
addr = {}
for l in open(kallsyms):
    p = l.split()
    if len(p) >= 3 and p[2] in ('_text', '__start___ksymtab', '__start___ksymtab_gpl', '__stop___ksymtab_gpl',
                                '__start___kcrctab', '__start___kcrctab_gpl', '__start___kcrctab_gpl_future'):
        addr[p[2]] = int(p[0], 16)
T = addr['_text']
def tab(ks, ke, kc, kind, res):
    n = (ke - ks) // 8
    for i in range(n):
        eo = ks - T + i * 8
        voff, noff = struct.unpack_from('<ii', img, eo)
        no = eo + 4 + noff
        name = img[no:img.index(b'\0', no)].decode()
        crc = struct.unpack_from('<I', img, kc - T + i * 4)[0]
        res.append((crc, name, kind))
    return n
res = []
n1 = tab(addr['__start___ksymtab'], addr['__start___ksymtab_gpl'], addr['__start___kcrctab'], 'EXPORT_SYMBOL', res)
n2 = tab(addr['__start___ksymtab_gpl'], addr['__stop___ksymtab_gpl'], addr['__start___kcrctab_gpl'], 'EXPORT_SYMBOL_GPL', res)
assert addr['__start___kcrctab_gpl'] - addr['__start___kcrctab'] == n1 * 4
assert addr['__start___kcrctab_gpl_future'] - addr['__start___kcrctab_gpl'] == n2 * 4
with open(out, 'w') as f:
    for crc, name, kind in res:
        f.write('0x%08x\t%s\tvmlinux\t%s\n' % (crc, name, kind))
print(n1, n2)
