#!/usr/bin/env python3
"""Run the ORIGINAL Riverrun DSP (i386 slice of Riverrun.vst 2.6.4) under Unicorn.

The constructors, `InitBuffers`, `Process`, `Run`, the AGC and every helper are
executed from the original machine code. Only the outside world is replaced:

* libc / Carbon imports (the `__IMPORT,__jump_table` stubs) are implemented in
  Python: a bump heap for allocations, libm for cos/pow/..., no-ops for logging;
* the parameter store (`vltute…::GetParamFloat/GetParamLong/SetParamLong/
  GetParameter`) is intercepted and answered from a Python dict;
* `IsBypassed` returns 0, `InitBlackout`/`Blackout` (the demo beep of an
  unauthorised copy) are skipped.

Functions returning a float/double on i386 do so on the x87 stack, so those
hooks jump to a tiny `fld [scratch]; ret` trampoline.

Usage:
    emulate_riverrun.py BINARY scenario.json out.f32

The scenario gives the sample rate, block size, number of blocks, an input
snippet (raw float32 file, oldest sample first) and parameter changes per
block. The output is interleaved stereo float32.
"""
import json
import math
import struct
import sys

from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_HOOK_CODE, UcError
from unicorn.x86_const import (UC_X86_REG_EAX, UC_X86_REG_EIP, UC_X86_REG_ESP)

# ---- addresses in the i386 slice ----------------------------------------
GYAYW_CTOR = 0x1E6C8
GYAYW_SIZE = 0xE8
INIT_BUFFERS = 0x2013C
PROCESS = 0x204C6
GET_PARAM_FLOAT = 0xE94C
GET_PARAM_LONG = 0xEBDE
SET_PARAM_LONG = 0xEB52
GET_PARAMETER = 0xED88
IS_BYPASSED = 0xE3DA
INIT_BLACKOUT = 0x22166
BLACKOUT = 0x21BE6

HEAP = 0x10000000
HEAP_SIZE = 0x08000000
STACK = 0x20000000
STACK_SIZE = 0x00100000
SCRATCH = 0x30000000  # trampolines + return values
SENTINEL = 0x30000F00  # return address that ends an emulated call
SETTINGS = 0x31000000  # fake parameter-store object (all calls hooked)


def load_macho(path):
    d = open(path, 'rb').read()
    magic, _cpu, _sub, _ft, ncmds, _sz, _fl = struct.unpack_from('<7I', d, 0)
    assert magic == 0xFEEDFACE, 'expects a thin i386 Mach-O'
    off, segs = 28, []
    for _ in range(ncmds):
        cmd, csz = struct.unpack_from('<2I', d, off)
        if cmd == 1:
            name = d[off + 8:off + 24].rstrip(b'\0').decode()
            va, vs, fo, fs = struct.unpack_from('<4I', d, off + 24)
            segs.append((name, va, vs, d[fo:fo + fs]))
        off += csz
    return segs


def jump_table(path):
    import subprocess
    out = subprocess.run(['otool', '-Iv', path], capture_output=True, text=True).stdout
    stubs, on = {}, False
    for line in out.splitlines():
        if 'jump_table' in line:
            on = True
            continue
        if on and line.startswith('0x'):
            p = line.split()
            stubs[int(p[0], 16)] = p[2] if len(p) > 2 else '?'
    return stubs


class Riverrun:
    def __init__(self, binary, sample_rate, params, n_in=1, n_out=2):
        self.uc = uc = Uc(UC_ARCH_X86, UC_MODE_32)
        for name, va, vs, data in load_macho(binary):
            if name == '__PAGEZERO' or vs == 0:
                continue
            uc.mem_map(va, (vs + 0xFFF) & ~0xFFF)
            uc.mem_write(va, data)
        uc.mem_map(HEAP, HEAP_SIZE)
        uc.mem_map(STACK, STACK_SIZE)
        uc.mem_map(SCRATCH, 0x1000)
        uc.mem_map(SETTINGS, 0x1000)
        uc.mem_write(SCRATCH + 0x100, bytes([0xDD, 0x05]) + struct.pack('<I', SCRATCH) + b'\xC3')  # fld qword
        uc.mem_write(SCRATCH + 0x110, bytes([0xD9, 0x05]) + struct.pack('<I', SCRATCH) + b'\xC3')  # fld dword
        uc.mem_write(SENTINEL, b'\xF4')
        self.brk = HEAP
        self.params = dict(params)
        self.set_log = []
        self.stubs = jump_table(binary)
        lo, hi = min(self.stubs), max(self.stubs) + 5
        uc.hook_add(UC_HOOK_CODE, self._stub, begin=lo, end=hi)
        for addr, fn in [(GET_PARAM_FLOAT, self._get_float), (GET_PARAM_LONG, self._get_long),
                         (SET_PARAM_LONG, self._set_long), (GET_PARAMETER, self._ret0),
                         (IS_BYPASSED, self._ret0), (INIT_BLACKOUT, self._ret0),
                         (BLACKOUT, self._ret0)]:
            uc.hook_add(UC_HOOK_CODE, fn, begin=addr, end=addr)

        self.this = self.alloc(GYAYW_SIZE)
        self.call(GYAYW_CTOR, [self.this])
        self.w64(self.this + 0x1C, sample_rate)
        self.w32(self.this + 0x24, n_in)
        self.w32(self.this + 0x28, n_out)
        self.w32(self.this + 0x2C, SETTINGS)
        self.n_in, self.n_out = n_in, n_out
        self.call(INIT_BUFFERS, [self.this])
        self.engine = self.r32(self.this + 0x80)

    # ---- memory helpers ----
    def alloc(self, n, align=16):
        self.brk = (self.brk + align - 1) & ~(align - 1)
        p = self.brk
        self.brk += max(n, 1)
        assert self.brk < HEAP + HEAP_SIZE, 'emulated heap exhausted'
        return p

    def r32(self, a):
        return struct.unpack('<I', self.uc.mem_read(a, 4))[0]

    def w32(self, a, v):
        self.uc.mem_write(a, struct.pack('<I', v & 0xFFFFFFFF))

    def w64(self, a, v):
        self.uc.mem_write(a, struct.pack('<d', v))

    def r64(self, a):
        return struct.unpack('<d', self.uc.mem_read(a, 8))[0]

    def arg(self, i):
        return self.r32(self.uc.reg_read(UC_X86_REG_ESP) + 4 + 4 * i)

    def argd(self, i):
        sp = self.uc.reg_read(UC_X86_REG_ESP) + 4 + 4 * i
        return struct.unpack('<d', self.uc.mem_read(sp, 8))[0]

    # ---- returning from a hooked function ----
    def ret(self, eax=None):
        uc = self.uc
        sp = uc.reg_read(UC_X86_REG_ESP)
        ra = self.r32(sp)
        if eax is not None:
            uc.reg_write(UC_X86_REG_EAX, eax & 0xFFFFFFFF)
        uc.reg_write(UC_X86_REG_ESP, sp + 4)
        uc.reg_write(UC_X86_REG_EIP, ra)

    def ret_double(self, v):
        self.uc.mem_write(SCRATCH, struct.pack('<d', v))
        self.uc.reg_write(UC_X86_REG_EIP, SCRATCH + 0x100)

    def ret_float(self, v):
        self.uc.mem_write(SCRATCH, struct.pack('<f', v))
        self.uc.reg_write(UC_X86_REG_EIP, SCRATCH + 0x110)

    def call(self, addr, args):
        uc = self.uc
        sp = STACK + STACK_SIZE - 0x1000
        sp -= 4 * len(args)
        for i, a in enumerate(args):
            self.w32(sp + 4 * i, a)
        sp -= 4
        self.w32(sp, SENTINEL)
        uc.reg_write(UC_X86_REG_ESP, sp)
        try:
            uc.emu_start(addr, SENTINEL)
        except UcError as e:
            eip = uc.reg_read(UC_X86_REG_EIP)
            raise RuntimeError(f'emulation fault at {eip:#x}: {e}') from None
        return uc.reg_read(UC_X86_REG_EAX)

    # ---- hooks ----
    def _ret0(self, uc, addr, size, ud):
        self.ret(0)

    def _get_float(self, uc, addr, size, ud):
        self.ret_float(float(self.params.get(self.arg(1), 0.0)))

    def _get_long(self, uc, addr, size, ud):
        self.ret(int(self.params.get(self.arg(1), 0)))

    def _set_long(self, uc, addr, size, ud):
        self.params[self.arg(1)] = struct.unpack('<i', struct.pack('<I', self.arg(2)))[0]
        self.ret(0)

    def _stub(self, uc, addr, size, ud):
        name = self.stubs.get(addr)
        if name is None:
            return
        f = getattr(self, 'imp' + name, None)
        if f is None:
            raise RuntimeError(f'unimplemented import {name}')
        f()

    # libc / Carbon -----------------------------------------------------
    def imp_cos(self): self.ret_double(math.cos(self.argd(0)))
    def imp_sin(self): self.ret_double(math.sin(self.argd(0)))
    def imp_log(self): self.ret_double(math.log(self.argd(0)))
    def imp_floor(self): self.ret_double(math.floor(self.argd(0)))
    def imp_ceil(self): self.ret_double(math.ceil(self.argd(0)))
    def imp_pow(self): self.ret_double(math.pow(self.argd(0), self.argd(2)))

    def _new(self):
        self.ret(self.alloc(self.arg(0)))
    imp__Znwm = imp__Znam = imp_malloc = imp_NewPtrClear = _new

    def imp_calloc(self):
        self.ret(self.alloc(self.arg(0) * self.arg(1)))

    def _noop(self):
        self.ret(0)
    imp__ZdlPv = imp__ZdaPv = imp_free = imp_DisposePtr = imp_MemError = _noop
    imp_Random = imp___Z4_LogPcz = imp___dynamic_cast = _noop

    def imp__ZNSsC2Ev(self):
        # libstdc++ copy-on-write std::string: point at an empty rep
        # (length, capacity, refcount, then the characters).
        rep = self.alloc(16)
        self.w32(self.arg(0), rep + 12)
        self.ret(self.arg(0))
    imp__ZNSsC1Ev = imp__ZNSsC2Ev
    imp__ZNSsD1Ev = imp__ZNSsD2Ev = _noop

    def imp_Microseconds(self):
        self.uc.mem_write(self.arg(0), b'\0' * 8)
        self.ret(0)

    def imp_memcpy(self):
        d, s, n = self.arg(0), self.arg(1), self.arg(2)
        if n:
            self.uc.mem_write(d, bytes(self.uc.mem_read(s, n)))
        self.ret(d)
    imp_memmove = imp_memcpy

    def imp_memset(self):
        d, c, n = self.arg(0), self.arg(1), self.arg(2)
        if n:
            self.uc.mem_write(d, bytes([c & 0xFF]) * n)
        self.ret(d)

    def f32(self, a):
        return struct.unpack('<f', self.uc.mem_read(a, 4))[0]

    def i32(self, a):
        return struct.unpack('<i', self.uc.mem_read(a, 4))[0]

    def trace(self):
        """The same fields, in the same order, as `Engine::trace` in Rust."""
        e, g, t = self.engine, self.r32(self.engine + 0x34), self.this
        return (f"t={self.r64(g+0x4c):.6f} last_t={self.r64(g+0x68):.6f} interval={self.r64(g+0x58):.6f} "
                f"new={self.r64(g+0x60):.6f} len={self.r64(g+0x40):.6f} pitch={self.r64(g+0x1c):.6f} "
                f"range={self.r64(g+0x24):.6f} fine={self.r64(g+0x2c):.6f} jit={self.i32(g+0x48)} "
                f"glass_pos={self.r64(e+0x74):.6f} glass_end={self.i32(e+0x7c)} glass={self.f32(t+0xd8):.6f} "
                f"nl={self.r32(t+0x88)} nr={self.r32(t+0x8c)} rng={self.r32(e+0x20):#x} "
                f"out_pos={self.i32(e+0x18)} shape={struct.unpack('<h', self.uc.mem_read(e+0x52, 2))[0]} "
                f"env_len={self.i32(e+0x6c)} agc={self.f32(t+0x7c):.9f} pk={self.f32(t+0x6c):.9f}")

    def set_transport(self, playing, changed, bpm, ppq, sample_pos):
        """Fill the tempo context the way VSTMissionHost::GetTimeInfo @ 0x26998 does."""
        ctx = self.r32(self.this + 0x34)
        sr = self.r64(ctx + 0x44)
        spq = sr / (bpm / 60.0)
        self.w64(ctx + 0x28, bpm)
        self.w64(ctx + 0x20, spq)
        self.w64(ctx + 0x18, spq / 24.0)
        self.w32(ctx + 0x3C, int(sample_pos))
        if changed:
            self.w32(ctx + 0x4C, 1 if playing else 0)
        self.w32(ctx + 0x50, 1 if changed else 0)
        self.w64(ctx + 0x54, ppq - math.trunc(ppq))

    # ---- engine access ----
    def snippet_ring(self):
        snip = self.r32(self.engine + 0x2C)
        return snip, self.r32(snip + 0xC), self.r32(snip + 0x10)

    def load_snippet(self, samples):
        _snip, buf, length = self.snippet_ring()
        data = list(samples[:length]) + [0.0] * max(0, length - len(samples))
        self.uc.mem_write(buf, struct.pack(f'<{length}f', *data))

    def process(self, n, inputs=None):
        ins = [self.alloc(4 * n) for _ in range(self.n_in)]
        outs = [self.alloc(4 * n) for _ in range(self.n_out)]
        if inputs:
            for p, x in zip(ins, inputs):
                self.uc.mem_write(p, struct.pack(f'<{n}f', *x))
        pin = self.alloc(4 * len(ins))
        pout = self.alloc(4 * len(outs))
        for i, p in enumerate(ins):
            self.w32(pin + 4 * i, p)
        for i, p in enumerate(outs):
            self.w32(pout + 4 * i, p)
        self.call(PROCESS, [self.this, pin, pout, n])
        res = [struct.unpack(f'<{n}f', self.uc.mem_read(p, 4 * n)) for p in outs]
        self.brk = ins[0]  # per-block scratch is reused
        return res


# Parameter ids of the original (tgxemus…::GetParameter @ 0x1d618).
PARAM_IDS = {
    'pitch': 0, 'speed': 1, 'record': 2, 'length': 3, 'random': 4, 'speed_random': 5,
    'walk': 6, 'envelope': 7, 'grid': 8, 'rhythmic': 9, 'mono_stereo': 10, 'glass': 11,
    'needle_l': 13, 'needle_r': 14, 'fine': 15,
}
DEFAULTS = {
    'pitch': 500.0, 'speed': 125.0, 'record': 0, 'length': 100.0, 'random': 0.0,
    'speed_random': 50.0, 'walk': 0.5, 'envelope': 2, 'grid': 1, 'rhythmic': 0,
    'mono_stereo': 93.0, 'glass': 150.0, 'needle_l': 20.0, 'needle_r': 270.0, 'fine': 1.0,
}


def main():
    binary, scen_path, out_path = sys.argv[1:4]
    trace = '--trace' in sys.argv
    sc = json.load(open(scen_path))
    p = dict(DEFAULTS)
    p.update(sc.get('params', {}))
    rr = Riverrun(binary, sc['sample_rate'], {PARAM_IDS[k]: v for k, v in p.items()})
    snippet = []
    if sc.get('snippet'):
        raw = open(sc['snippet'], 'rb').read()
        snippet = struct.unpack(f'<{len(raw) // 4}f', raw)
        rr.load_snippet(snippet)
    changes = {int(k): v for k, v in sc.get('changes', {}).items()}
    n = sc['block']
    out = bytearray()
    for b in range(sc['blocks']):
        for k, v in changes.get(b, {}).items():
            rr.params[PARAM_IDS[k]] = v
        live = None
        if sc.get('live_input') and snippet:
            off = (b * n) % (len(snippet) - n)
            live = [snippet[off:off + n]]
        tr = sc.get('transport')
        if tr:
            pos = b * n
            ppq = tr.get('start_ppq', 0.0) + pos / (sc['sample_rate'] * 60.0 / tr['bpm'])
            rr.set_transport(tr['playing'], b == 0, tr['bpm'], ppq, pos)
        l, r = rr.process(n, live)
        if trace:
            print(f'{b:4d} {rr.trace()}')
        for a, c in zip(l, r):
            out += struct.pack('<2f', a, c)
    open(out_path, 'wb').write(out)
    print(f'{sc["blocks"]} blocks of {n}: rng={rr.r32(rr.engine + 0x20):#x} '
          f'agc={struct.unpack("<f", rr.uc.mem_read(rr.this + 0x7c, 4))[0]:.6f}')


if __name__ == '__main__':
    main()
