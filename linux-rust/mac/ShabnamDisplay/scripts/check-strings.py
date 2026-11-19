#!/usr/bin/env python3
"""Checks that every UI string used in the sources has an entry in each Localizable.strings,
and that no entry is unused. Run from the project root: python3 scripts/check-strings.py"""
import glob
import re
import sys

keys = []
for path in sorted(glob.glob("Sources/**/*.swift", recursive=True)):
    for match in re.finditer(r'\bL\("((?:[^"\\]|\\.)*)"', open(path, encoding="utf-8").read()):
        if match.group(1) not in keys:
            keys.append(match.group(1))

failed = False
for table in sorted(glob.glob("Resources/*.lproj/Localizable.strings")):
    text = open(table, encoding="utf-8").read()
    have = set(re.findall(r'^"((?:[^"\\]|\\.)*)" = ', text, re.M))
    missing = [k for k in keys if k not in have]
    unused = [k for k in have if k not in keys]
    print(f"{table}: {len(have)} entries, {len(missing)} missing, {len(unused)} unused")
    for k in missing:
        print("  missing:", k)
    for k in unused:
        print("  unused: ", k)
    failed = failed or bool(missing or unused)
sys.exit(1 if failed else 0)
