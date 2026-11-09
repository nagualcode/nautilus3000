#!/usr/bin/env python3
"""Run the ORIGINAL Deep Phase Nine DSP (i386 slice of DeepPhaseNine.vst 2.6.4)
under Unicorn.

Unlike the Riverrun harness, the parameter store is the original one: every
parameter object is created by the binary's own `GetParameter` /
`GetPrivateParameter`, and only `GetParamFromMap` (the std::map lookup) is
answered from Python. That matters here because the phaser writes its LFO into
a private parameter and reads it back, relying on the store's clamping and
type conversions.

    emulate_deepphasenine.py BINARY scenario.json out.f32 [--trace]

Output: interleaved float32, one or two channels.
"""
import json
import math
import os
import struct
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..', 'tools'))
from mission_emu import Mission  # noqa: E402

CTOR = 0x1AC90
SIZE = 0xCC
INIT_BUFFERS = 0x1E104
PROCESS = 0x1C784
GET_PARAMETER = 0x19E56
GET_PRIVATE_PARAMETER = 0x1A60C
GET_PARAM_FROM_MAP = 0xE758
GET_SAMPLE_RATE = 0xE67A
IS_BYPASSED = 0xE54A
OPTION_KEY_DOWN = 0x6580
INIT_BLACKOUT = 0x1EC6E
BLACKOUT = 0x1E6EE

PARAM_IDS = {
    'notches': 0, 'feedback': 1, 'low': 2, 'high': 3, 'spread': 4, 'wet_dry': 5,
    'lfo_choice': 6, 'lfo_speed': 7, 'oneshot_end': 8, 'oneshot_dir': 9, 'sync': 10,
    'stereo_offset': 11,
}
LONG_PARAMS = {0, 6, 7, 9, 10, 11}


class DeepPhaseNine(Mission):
    def __init__(self, binary, sample_rate, variant, values):
        super().__init__(binary)
        self.sr = sample_rate
        self.hook(GET_PARAM_FROM_MAP, lambda: self.ret(self.params.get(self.arg(1), 0)))
        self.hook(GET_SAMPLE_RATE, lambda: self.ret_double(self.sr))
        for a in (IS_BYPASSED, OPTION_KEY_DOWN, INIT_BLACKOUT, BLACKOUT):
            self.hook(a, lambda: self.ret(0))

        # Parameter objects, made by the binary itself.
        desc = self.alloc(0x100)
        self.params = {}
        for i in range(12):
            self.params[i] = self.call(GET_PARAMETER, [desc, i])
        for k in range(11):
            p = self.call(GET_PRIVATE_PARAMETER, [desc, k])
            self.params[0xC + k] = p
        self.settings = self.alloc(0x100)
        notify = self.make_vtable_object({})
        self.w32(self.settings + 0x18, notify)
        get_settings = self.code(b'\xB8' + struct.pack('<I', self.settings) + b'\xC3')
        host = self.make_vtable_object({0x84: get_settings})

        self.this = self.alloc(SIZE)
        self.call(CTOR, [self.this])
        self.w32(self.this + 0x18, variant)
        self.w64(self.this + 0x1C, sample_rate)
        n_in, n_out = {0: (1, 1), 1: (1, 2), 2: (2, 2)}[variant]
        self.n_in, self.n_out = n_in, n_out
        self.w32(self.this + 0x24, n_in)
        self.w32(self.this + 0x28, n_out)
        self.w32(self.this + 0x2C, self.settings)
        self.w32(self.this + 0x38, host)
        self.set_values(values)
        self.call(INIT_BUFFERS, [self.this])

    def set_values(self, values):
        for name, v in values.items():
            pid = PARAM_IDS[name]
            obj = self.params[pid]
            if pid in LONG_PARAMS:
                self.w32(obj + 0x78, int(v))
            else:
                self.wf32(obj + 0x78, float(v))

    def set_transport(self, playing, changed, bpm, ppq):
        ctx = self.r32(self.this + 0x34)
        spq = self.r64(ctx + 0x44) / (bpm / 60.0)
        self.w64(ctx + 0x28, bpm)
        self.w64(ctx + 0x20, spq)
        self.w64(ctx + 0x18, spq / 24.0)
        frac = ppq - math.trunc(ppq)
        self.w32(ctx + 0x38, int((1.0 - frac) * spq))
        if changed:
            self.w32(ctx + 0x4C, 1 if playing else 0)
        self.w32(ctx + 0x50, 1 if changed else 0)
        self.w64(ctx + 0x54, frac)

    def process(self, inputs):
        n = len(inputs[0])
        ins = [self.alloc(4 * n) for _ in range(self.n_in)]
        outs = [self.alloc(4 * n) for _ in range(self.n_out)]
        for p, x in zip(ins, inputs):
            self.uc.mem_write(p, struct.pack(f'<{n}f', *x))
        pin, pout = self.alloc(16), self.alloc(16)
        for i, p in enumerate(ins):
            self.w32(pin + 4 * i, p)
        for i, p in enumerate(outs):
            self.w32(pout + 4 * i, p)
        self.call(PROCESS, [self.this, pin, pout, n])
        res = [struct.unpack(f'<{n}f', self.uc.mem_read(p, 4 * n)) for p in outs]
        self.brk = ins[0]
        return res

    def trace(self):
        p = self.r32(self.this + 0x80)
        lfo = self.f32(self.params[0xC] + 0x78)
        d = self.i32(self.params[0x12] + 0x78)
        return (f"phase={self.f32(p + 0xc):.7f} dir={self.i32(p + 0x20)} rate={self.f32(p + 0x1c):.9f} "
                f"lo={self.f32(p):.7f} hi={self.f32(p + 4):.7f} fb={self.f32(p + 8):.6f} "
                f"tick={self.i32(p + 0x638)} lfo_pos={lfo:.7f} lfo_dir={d} rng={self.r32(p + 0x674):#x} "
                f"fbl={self.f32(p + 0x30):.7f} fbr={self.f32(p + 0x34):.7f} agc={self.f32(self.this + 0x7c):.7f}")


DEFAULTS = {
    'notches': 3, 'feedback': 0.8, 'low': 100.0, 'high': 4000.0, 'spread': 35.0, 'wet_dry': 1.0,
    'lfo_choice': 1, 'lfo_speed': 21, 'oneshot_end': 0.5, 'oneshot_dir': -1, 'sync': 0,
    'stereo_offset': 0,
}


def main():
    binary, scen, out_path = sys.argv[1:4]
    trace = '--trace' in sys.argv
    sc = json.load(open(scen))
    variant = {'mono': 0, 'mono_to_stereo': 1, 'stereo': 2}[sc.get('layout', 'stereo')]
    values = dict(DEFAULTS)
    values.update(sc.get('params', {}))
    dpn = DeepPhaseNine(binary, sc['sample_rate'], variant, values)
    raw = open(sc['input'], 'rb').read()
    sig = struct.unpack(f'<{len(raw) // 4}f', raw)
    left, right = sig[0::2], sig[1::2]
    n = sc['block']
    changes = {int(k): v for k, v in sc.get('changes', {}).items()}
    out = bytearray()
    for b in range(sc['blocks']):
        if b in changes:
            dpn.set_values(changes[b])
        tr = sc.get('transport')
        if tr:
            pos = b * n
            ppq = tr.get('start_ppq', 0.0) + pos / (sc['sample_rate'] * 60.0 / tr['bpm'])
            dpn.set_transport(tr['playing'], b == 0, tr['bpm'], ppq)
        o = (b * n) % (len(left) - n)
        ins = [left[o:o + n]] if dpn.n_in == 1 else [left[o:o + n], right[o:o + n]]
        res = dpn.process(ins)
        if trace:
            print(f'{b:4d} {dpn.trace()}')
        for frame in zip(*res):
            out += struct.pack(f'<{len(frame)}f', *frame)
    open(out_path, 'wb').write(out)
    print(f'{sc["blocks"]} blocks: mxcsr={dpn.mxcsr():#x} {dpn.trace()}')


if __name__ == '__main__':
    main()
