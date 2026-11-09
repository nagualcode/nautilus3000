#!/usr/bin/env python3
"""Annotate `otool -tvV` output of an i386 Mach-O: resolve PIC constants.

GCC's i386 PIC code loads constants as `k(%ebx)` after `call X; X: popl %ebx`.
For a function range this prints each instruction and, for every `k(%ebx)`
operand, the address and the value read there (float for *ss, double for *sd).

    annotate_asm.py BINARY START END        (hex addresses)
"""
import os
import re
import struct
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from macho import MachO  # noqa: E402


def main():
    path, start, end = sys.argv[1], int(sys.argv[2], 16), int(sys.argv[3], 16)
    m = MachO(path)
    out = subprocess.run(['otool', '-tvV', path], capture_output=True, text=True).stdout
    ebx = None
    pending_call = None
    for line in out.splitlines():
        p = line.split('\t')
        try:
            a = int(p[0], 16)
        except ValueError:
            continue
        if not start <= a < end:
            continue
        ins = '\t'.join(p[1:])
        if pending_call is not None and a == pending_call and 'popl\t%ebx' in ins:
            ebx = a
        mc = re.match(r'calll\t0x([0-9a-f]+)$', ins)
        pending_call = int(mc.group(1), 16) if mc else None
        note = ''
        mm = re.search(r'(-?0x[0-9a-f]+)\(%ebx\)', ins)
        if mm and ebx is not None and ins.startswith('lea'):
            va = ebx + int(mm.group(1), 16)
            raw = m.read(va, 8)
            note = (f'   ; &{va:#x}: float {struct.unpack("<f", raw[:4])[0]!r}'
                    f' / double {struct.unpack("<d", raw)[0]!r}')
        elif mm and ebx is not None:
            va = ebx + int(mm.group(1), 16)
            raw = m.read(va, 8)
            op = ins.split('\t')[0]
            if op.endswith('sd') or op in ('fldl', 'faddl', 'fmull'):
                note = f'   ; [{va:#x}] = {struct.unpack("<d", raw)[0]!r} (double)'
            elif op.endswith('ss') or op.startswith('movss'):
                note = f'   ; [{va:#x}] = {struct.unpack("<f", raw[:4])[0]!r} (float)'
            else:
                note = f'   ; [{va:#x}] = {raw[:4].hex()}'
        print(f'{a:08x}  {ins}{note}')


if __name__ == '__main__':
    main()
