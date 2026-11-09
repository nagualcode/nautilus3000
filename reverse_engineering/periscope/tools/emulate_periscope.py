#!/usr/bin/env python3
"""Run the ORIGINAL PeriScope DSP (i386 slice of Periscope.vst 2.6.4) under
Unicorn.

The parameter objects are the binary's own (`GetParameter` /
`GetPrivateParameter`); only the `std::map` lookup is answered from Python.
The FFT is vecLib's: `create_fftsetup` / `fft_zrip` are forwarded to the
real Accelerate framework through ctypes (the very routines the Rust port
calls), `ctoz` / `ztoc` are plain data moves done here.

    emulate_periscope.py BINARY scenario.json out.f32

Output: interleaved float32, one or two channels.
"""
import ctypes
import json
import math
import os
import struct
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..', 'tools'))
from mission_emu import Mission  # noqa: E402

CTOR = 0x1A198
SIZE = 0x2AC
INIT_BUFFERS = 0x1A83E
PROCESS = 0x1B674
GET_PARAMETER = 0x1C208
GET_PRIVATE_PARAMETER = 0x1C68C
GET_PARAM_FROM_MAP = 0xE3B8
GET_SAMPLE_RATE = 0xE2DA
IS_BYPASSED = 0xE1AA
OPTION_KEY_DOWN = 0x61E0
INIT_BLACKOUT = 0x1BDE2
BLACKOUT = 0x1BBAC
GLOBAL_INIT_THUMBS = 0x156A    # __GLOBAL__I_gThumbBreakPoints (fills the fader → gain table)
THUMB_BREAKPOINTS = 0x34FE0

FLOAT_PARAMS = {0, 1, 2, 3, 5}

_acc = ctypes.CDLL('/System/Library/Frameworks/Accelerate.framework/Accelerate')
_acc.vDSP_create_fftsetup.restype = ctypes.c_void_p
_acc.vDSP_create_fftsetup.argtypes = [ctypes.c_size_t, ctypes.c_int]


class _Split(ctypes.Structure):
    _fields_ = [('realp', ctypes.POINTER(ctypes.c_float)), ('imagp', ctypes.POINTER(ctypes.c_float))]


_acc.vDSP_fft_zrip.argtypes = [ctypes.c_void_p, ctypes.POINTER(_Split), ctypes.c_long, ctypes.c_size_t, ctypes.c_int]


_keep = []


def _aligned(n):
    raw = ctypes.create_string_buffer(4 * n + 64)
    _keep.append(raw)
    if len(_keep) > 64:
        del _keep[:32]
    a = (ctypes.addressof(raw) + 63) & ~63
    return ctypes.c_void_p(a)


class Periscope(Mission):
    def __init__(self, binary, sample_rate, channels, buffer, values):
        super().__init__(binary)
        self.sr = sample_rate
        self.setups = {}
        self.hook(GET_PARAM_FROM_MAP, lambda: self.ret(self.params.get(self.arg(1), 0)))
        self.hook(GET_SAMPLE_RATE, lambda: self.ret_double(self.sr))
        for a in (IS_BYPASSED, OPTION_KEY_DOWN, INIT_BLACKOUT, BLACKOUT):
            self.hook(a, lambda: self.ret(0))

        self.call(GLOBAL_INIT_THUMBS, [])
        desc = self.alloc(0x100)
        self.params = {}
        for i in range(0x26):
            self.params[i] = self.call(GET_PARAMETER, [desc, i])
        for k in range(12):
            self.params[0x26 + k] = self.call(GET_PRIVATE_PARAMETER, [desc, k])
        self.settings = self.alloc(0x100)
        # The host: an automatable parameter set by the plug-in itself (the
        # needles pushed apart in UpdateFreqs) is only *sent to the host*
        # (vtable +0x3c); a VST host writes it straight back (setParameter).
        self.automated = []
        automate = self.code(b'\xC3')
        self.hook(automate, lambda: (self.automated.append((self.arg(1), self.arg(2))), self.ret()))
        notify = self.make_vtable_object({0x3C: automate})
        self.w32(self.settings + 0x18, notify)
        get_settings = self.code(b'\xB8' + struct.pack('<I', self.settings) + b'\xC3')
        host = self.make_vtable_object({0x84: get_settings})

        self.this = self.alloc(SIZE)
        self.call(CTOR, [self.this])
        self.n = channels
        self.w64(self.this + 0x1C, sample_rate)
        self.w32(self.this + 0x24, channels)
        self.w32(self.this + 0x28, channels)
        self.w32(self.this + 0x2C, self.settings)
        self.w32(self.this + 0x30, buffer)
        self.w32(self.this + 0x38, host)
        self.set_values(values)
        self.call(INIT_BUFFERS, [self.this])

    def set_values(self, values):
        for name, v in values.items():
            if name == 'faders':
                for k, f in enumerate(v):
                    self.w32(self.params[6 + k] + 0x78, int(f))
                continue
            pid = {'low': 2, 'high': 3}[name]
            self.wf32(self.params[pid] + 0x78, float(v))

    def needles(self):
        return self.f32(self.params[2] + 0x78), self.f32(self.params[3] + 0x78)

    # ---- vecLib ----
    def imp_create_fftsetup(self):
        log2n = self.arg(0)
        h = len(self.setups) + 1
        self.setups[h] = (_acc.vDSP_create_fftsetup(log2n, self.arg(1)), log2n)
        self.ret(h)

    def imp_ctoz(self):
        c, ic, z, iz, n = (self.arg(i) for i in range(5))
        assert ic == 2 and iz == 1
        data = struct.unpack(f'<{2 * n}f', self.uc.mem_read(c, 8 * n))
        self.uc.mem_write(self.r32(z), struct.pack(f'<{n}f', *data[0::2]))
        self.uc.mem_write(self.r32(z + 4), struct.pack(f'<{n}f', *data[1::2]))
        self.ret(0)

    def imp_ztoc(self):
        z, iz, c, ic, n = (self.arg(i) for i in range(5))
        assert ic == 2 and iz == 1
        re = struct.unpack(f'<{n}f', self.uc.mem_read(self.r32(z), 4 * n))
        im = struct.unpack(f'<{n}f', self.uc.mem_read(self.r32(z + 4), 4 * n))
        out = [0.0] * (2 * n)
        out[0::2], out[1::2] = re, im
        self.uc.mem_write(c, struct.pack(f'<{2 * n}f', *out))
        self.ret(0)

    def imp_fft_zrip(self):
        h, z, stride, log2n, d = (self.arg(i) for i in range(5))
        setup, _ = self.setups[h]
        n = 1 << (log2n - 1)
        # vDSP picks its kernel by buffer alignment (and the kernels round
        # differently): always 64-byte aligned, like the Rust port.
        re, im = _aligned(n), _aligned(n)
        ctypes.memmove(re, bytes(self.uc.mem_read(self.r32(z), 4 * n)), 4 * n)
        ctypes.memmove(im, bytes(self.uc.mem_read(self.r32(z + 4), 4 * n)), 4 * n)
        sp = _Split(ctypes.cast(re, ctypes.POINTER(ctypes.c_float)), ctypes.cast(im, ctypes.POINTER(ctypes.c_float)))
        _acc.vDSP_fft_zrip(setup, ctypes.byref(sp), stride, log2n, ctypes.c_int32(d).value)
        self.uc.mem_write(self.r32(z), ctypes.string_at(re, 4 * n))
        self.uc.mem_write(self.r32(z + 4), ctypes.string_at(im, 4 * n))
        self.ret(0)

    def imp_destroy_fftsetup(self):
        self.ret(0)

    def imp_puts(self):
        self.ret(0)

    def imp_Gestalt(self):
        self.w32(self.arg(1), 0)
        self.ret(0)

    # ---- libm, float versions correctly rounded through double ----
    def imp_exp(self): self.ret_double(math.exp(self.argd(0)))
    def imp_expf(self): self.ret_float(math.exp(self.argf(0)))
    def imp_logf(self): self.ret_float(math.log(self.argf(0)))
    def imp_floorf(self): self.ret_float(math.floor(self.argf(0)))
    def imp_powf(self): self.ret_float(math.pow(self.argf(0), self.argf(1)))
    def imp_log10(self): self.ret_double(math.log10(self.argd(0)))
    def imp_log10f(self): self.ret_float(math.log10(self.argf(0)))

    def process(self, inputs):
        n = len(inputs[0])
        ins = [self.alloc(4 * n) for _ in range(self.n)]
        outs = [self.alloc(4 * n) for _ in range(self.n)]
        for p, x in zip(ins, inputs):
            self.uc.mem_write(p, struct.pack(f'<{n}f', *x))
        pin, pout = self.alloc(16), self.alloc(16)
        for i, p in enumerate(ins):
            self.w32(pin + 4 * i, p)
        for i, p in enumerate(outs):
            self.w32(pout + 4 * i, p)
        self.call(PROCESS, [self.this, pin, pout, n])
        for pid, bits in self.automated:
            obj = self.params[pid]
            self.call(self.r32(self.r32(obj) + 0x48), [obj, bits])   # SetFloatValue
        self.automated.clear()
        res = [struct.unpack(f'<{n}f', self.uc.mem_read(p, 4 * n)) for p in outs]
        self.brk = ins[0]
        return res

    def freqs(self):
        p = self.r32(self.params[0x27] + 0x78)
        return struct.unpack('<32f', self.uc.mem_read(p, 128))


DEFAULTS = {'low': 40.0, 'high': 300.0, 'faders': [135] * 32}


def main():
    binary, scen, out_path = sys.argv[1:4]
    sc = json.load(open(scen))
    values = dict(DEFAULTS)
    values.update(sc.get('params', {}))
    ch = sc.get('channels', 2)
    n = sc['block']
    ps = Periscope(binary, sc['sample_rate'], ch, sc.get('buffer', n), values)
    raw = open(sc['input'], 'rb').read()
    sig = struct.unpack(f'<{len(raw) // 4}f', raw)
    left, right = sig[0::2], sig[1::2]
    changes = {int(k): v for k, v in sc.get('changes', {}).items()}
    out = bytearray()
    for b in range(sc['blocks']):
        if b in changes:
            ps.set_values(changes[b])
        o = (b * n) % (len(left) - n)
        ins = [left[o:o + n]] if ch == 1 else [left[o:o + n], right[o:o + n]]
        res = ps.process(ins)
        for frame in zip(*res):
            out += struct.pack(f'<{len(frame)}f', *frame)
    open(out_path, 'wb').write(out)
    lo, hi = ps.needles()
    print(f'{sc["blocks"]} blocks: needles {lo:.6f} {hi:.6f} freqs {ps.freqs()[:3]} … mxcsr={ps.mxcsr():#x}')


if __name__ == '__main__':
    main()
