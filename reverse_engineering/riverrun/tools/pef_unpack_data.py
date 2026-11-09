#!/usr/bin/env python3
"""Expand the pattern-initialised data section of a PEF container.

    pef_unpack_data.py FILE.pef OUT.bin
"""
import struct
import sys


def count(d, p):
    v = 0
    while True:
        b = d[p]
        p += 1
        v = (v << 7) | (b & 0x7F)
        if not b & 0x80:
            return v, p


def unpack(d, size):
    out = bytearray()
    p = 0
    while p < len(d) and len(out) < size:
        b = d[p]
        p += 1
        op, n = b >> 5, b & 0x1F
        if n == 0:
            n, p = count(d, p)
        if op == 0:
            out += bytes(n)
        elif op == 1:
            out += d[p:p + n]
            p += n
        elif op == 2:
            r, p = count(d, p)
            chunk = d[p:p + n]
            p += n
            out += chunk * (r + 1)
        elif op == 3:
            k = n
            m, p = count(d, p)
            r, p = count(d, p)
            common = d[p:p + k]
            p += k
            for _ in range(r):
                out += common + d[p:p + m]
                p += m
            out += common
        elif op == 4:
            k = n
            m, p = count(d, p)
            r, p = count(d, p)
            for _ in range(r):
                out += bytes(k) + d[p:p + m]
                p += m
            out += bytes(k)
        else:
            raise ValueError(f'opcode {op}')
    return bytes(out)


def main():
    d = open(sys.argv[1], 'rb').read()
    nsec = struct.unpack('>H', d[0x20:0x22])[0]
    for i in range(nsec):
        o = 0x28 + 28 * i
        _n, _a, exec_sz, init_sz, raw_sz, off, kind = struct.unpack('>iIIIIIB', d[o:o + 25])
        if kind == 2:
            data = unpack(d[off:off + raw_sz], init_sz)
            open(sys.argv[2], 'wb').write(data + bytes(exec_sz - len(data)))
            print(f'data section: {raw_sz} packed -> {len(data)} (+{exec_sz - len(data)} zero)')


if __name__ == '__main__':
    main()
