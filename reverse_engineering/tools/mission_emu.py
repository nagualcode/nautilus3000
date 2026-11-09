"""Shared Unicorn harness for the i386 Nautilus Bundle binaries (Audio Ease's
"Mission" framework, 2.6.4).

Loads a thin i386 Mach-O bundle at its link address, implements the libc /
Carbon imports of `__IMPORT,__jump_table` in Python, and offers helpers to
call functions, return integers or x87 floats from hooks, and to read/write
memory. Plug-in specific drivers (see `deepphasenine/tools/`) build on it.
"""
import math
import struct
import subprocess

from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_HOOK_CODE, UcError
from unicorn.x86_const import UC_X86_REG_EAX, UC_X86_REG_EIP, UC_X86_REG_ESP, UC_X86_REG_MXCSR

HEAP = 0x10000000
HEAP_SIZE = 0x08000000
STACK = 0x20000000
STACK_SIZE = 0x00100000
SCRATCH = 0x30000000      # x87 trampolines, return values, small stubs
SENTINEL = 0x30000F00     # return address that ends an emulated call
OBJECTS = 0x31000000      # fake host objects


def load_macho(path):
    d = open(path, 'rb').read()
    magic, _c, _s, _f, ncmds, _sz, _fl = struct.unpack_from('<7I', d, 0)
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


def symbols(path):
    out = subprocess.run(['nm', '-n', path], capture_output=True, text=True).stdout
    syms = {}
    for line in out.splitlines():
        p = line.split()
        if len(p) >= 3:
            syms[p[2]] = int(p[0], 16)
    return syms


def non_lazy_pointers(path):
    """`__IMPORT,__pointers` slots and the symbol each one names."""
    out = subprocess.run(['otool', '-Iv', path], capture_output=True, text=True).stdout
    slots, on = {}, False
    for line in out.splitlines():
        if '__pointers' in line:
            on = True
            continue
        if 'jump_table' in line:
            on = False
        if on and line.startswith('0x'):
            p = line.split()
            if len(p) > 2:
                slots[int(p[0], 16)] = p[2]
    return slots


def jump_table(path):
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


class Mission:
    def __init__(self, binary):
        self.uc = uc = Uc(UC_ARCH_X86, UC_MODE_32)
        for name, va, vs, data in load_macho(binary):
            if vs:
                uc.mem_map(va, (vs + 0xFFF) & ~0xFFF)
                uc.mem_write(va, data)
        uc.mem_map(HEAP, HEAP_SIZE)
        uc.mem_map(STACK, STACK_SIZE)
        uc.mem_map(SCRATCH, 0x1000)
        uc.mem_map(OBJECTS, 0x10000)
        uc.mem_write(SCRATCH + 0x100, b'\xDD\x05' + struct.pack('<I', SCRATCH) + b'\xC3')  # fld qword; ret
        uc.mem_write(SCRATCH + 0x110, b'\xD9\x05' + struct.pack('<I', SCRATCH) + b'\xC3')  # fld dword; ret
        uc.mem_write(SCRATCH + 0x120, b'\xC3')                                             # ret
        uc.mem_write(SENTINEL, b'\xF4')
        self.brk = HEAP
        self.syms = symbols(binary)
        self.stubs = jump_table(binary)
        # Bind non-lazy pointers to symbols the bundle defines itself (dyld's
        # job: e.g. weak C++ vtables).
        for slot, name in non_lazy_pointers(binary).items():
            if name in self.syms and self.r32(slot) == 0:
                self.w32(slot, self.syms[name])
        lo, hi = min(self.stubs), max(self.stubs) + 5
        uc.hook_add(UC_HOOK_CODE, self._stub, begin=lo, end=hi)
        self.obj_brk = OBJECTS + 0x100

    # ---- symbols & hooks ----
    def sym(self, mangled):
        return self.syms[mangled]

    def hook(self, addr, fn):
        self.uc.hook_add(UC_HOOK_CODE, lambda uc, a, s, u: fn(), begin=addr, end=addr)

    def make_vtable_object(self, entries, size=0x100):
        """A fake C++ object: returns its address; `entries` maps vtable byte
        offsets to code addresses, every other slot points at `ret`."""
        vt = self.obj_brk
        self.obj_brk += 0x400
        for o in range(0, 0x400, 4):
            self.w32(vt + o, entries.get(o, SCRATCH + 0x120))
        obj = self.obj_brk
        self.obj_brk += size
        self.w32(obj, vt)
        return obj

    def code(self, blob):
        """Places a small code snippet in scratch space and returns its address."""
        a = self.obj_brk
        self.obj_brk += (len(blob) + 15) & ~15
        self.uc.mem_write(a, blob)
        return a

    # ---- memory ----
    def alloc(self, n, align=16):
        self.brk = (self.brk + align - 1) & ~(align - 1)
        p = self.brk
        self.brk += max(n, 1)
        assert self.brk < HEAP + HEAP_SIZE, 'emulated heap exhausted'
        return p

    def r32(self, a):
        return struct.unpack('<I', self.uc.mem_read(a, 4))[0]

    def i32(self, a):
        return struct.unpack('<i', self.uc.mem_read(a, 4))[0]

    def f32(self, a):
        return struct.unpack('<f', self.uc.mem_read(a, 4))[0]

    def r64(self, a):
        return struct.unpack('<d', self.uc.mem_read(a, 8))[0]

    def w32(self, a, v):
        self.uc.mem_write(a, struct.pack('<I', v & 0xFFFFFFFF))

    def wf32(self, a, v):
        self.uc.mem_write(a, struct.pack('<f', v))

    def w64(self, a, v):
        self.uc.mem_write(a, struct.pack('<d', v))

    def arg(self, i):
        return self.r32(self.uc.reg_read(UC_X86_REG_ESP) + 4 + 4 * i)

    def argf(self, i):
        return struct.unpack('<f', struct.pack('<I', self.arg(i)))[0]

    def argd(self, i):
        sp = self.uc.reg_read(UC_X86_REG_ESP) + 4 + 4 * i
        return struct.unpack('<d', self.uc.mem_read(sp, 8))[0]

    # ---- returning from hooks ----
    def ret(self, eax=None):
        sp = self.uc.reg_read(UC_X86_REG_ESP)
        ra = self.r32(sp)
        if eax is not None:
            self.uc.reg_write(UC_X86_REG_EAX, eax & 0xFFFFFFFF)
        self.uc.reg_write(UC_X86_REG_ESP, sp + 4)
        self.uc.reg_write(UC_X86_REG_EIP, ra)

    def ret_double(self, v):
        self.uc.mem_write(SCRATCH, struct.pack('<d', v))
        self.uc.reg_write(UC_X86_REG_EIP, SCRATCH + 0x100)

    def ret_float(self, v):
        self.uc.mem_write(SCRATCH, struct.pack('<f', v))
        self.uc.reg_write(UC_X86_REG_EIP, SCRATCH + 0x110)

    def call(self, addr, args):
        sp = STACK + STACK_SIZE - 0x1000
        sp -= 4 * len(args)
        for i, a in enumerate(args):
            self.w32(sp + 4 * i, a)
        sp -= 4
        self.w32(sp, SENTINEL)
        self.uc.reg_write(UC_X86_REG_ESP, sp)
        try:
            self.uc.emu_start(addr, SENTINEL)
        except UcError as e:
            eip = self.uc.reg_read(UC_X86_REG_EIP)
            raise RuntimeError(f'emulation fault at {eip:#x}: {e}') from None
        return self.uc.reg_read(UC_X86_REG_EAX)

    def mxcsr(self):
        return self.uc.reg_read(UC_X86_REG_MXCSR)

    # ---- imports ----
    def _stub(self, uc, addr, size, ud):
        name = self.stubs.get(addr)
        if name is None:
            return
        f = getattr(self, 'imp' + name, None)
        if f is None:
            raise RuntimeError(f'unimplemented import {name}')
        f()

    def imp_cos(self): self.ret_double(math.cos(self.argd(0)))
    def imp_sin(self): self.ret_double(math.sin(self.argd(0)))
    def imp_acos(self): self.ret_double(math.acos(self.argd(0)))
    def imp_log(self): self.ret_double(math.log(self.argd(0)))
    def imp_floor(self): self.ret_double(math.floor(self.argd(0)))
    def imp_ceil(self): self.ret_double(math.ceil(self.argd(0)))
    def imp_pow(self): self.ret_double(math.pow(self.argd(0), self.argd(2)))
    # float libm: correctly rounded through double, like the Rust port
    def imp_cosf(self): self.ret_float(math.cos(self.argf(0)))
    def imp_sinf(self): self.ret_float(math.sin(self.argf(0)))

    def _new(self):
        self.ret(self.alloc(self.arg(0)))
    imp__Znwm = imp__Znam = imp_malloc = imp_NewPtrClear = imp_NewPtr = _new

    def imp_calloc(self):
        self.ret(self.alloc(self.arg(0) * self.arg(1)))

    def _noop(self):
        self.ret(0)
    imp__ZdlPv = imp__ZdaPv = imp_free = imp_DisposePtr = imp_MemError = _noop
    imp_Random = imp___Z4_LogPcz = imp___dynamic_cast = _noop

    def imp__ZNSsC2Ev(self):
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

    def imp_strncpy(self):
        d, s, n = self.arg(0), self.arg(1), self.arg(2)
        src = bytes(self.uc.mem_read(s, n))
        z = src.find(b'\0')
        src = src[:z] if z >= 0 else src
        self.uc.mem_write(d, (src + b'\0' * n)[:n])
        self.ret(d)

    def imp_strcpy(self):
        d, s = self.arg(0), self.arg(1)
        src = bytes(self.uc.mem_read(s, 256))
        src = src[:src.find(b'\0') + 1]
        self.uc.mem_write(d, src)
        self.ret(d)

    def imp_strlen(self):
        s = bytes(self.uc.mem_read(self.arg(0), 4096))
        self.ret(s.find(b'\0'))
