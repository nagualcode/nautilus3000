#!/usr/bin/env python3
"""Differential test: the original PeriScope (emulated) against the Rust
engine, bit for bit.

    differential_test.py ORIGINAL_I386_BINARY path/to/periscope-render WORKDIR [name-filter]
"""
import json
import os
import random
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, '..', '..', 'deepphasenine', 'tools'))
from differential_test import make_input, run, compare  # noqa: E402


def curve(fn):
    return [max(0, min(265, int(round(fn(k))))) for k in range(32)]


def scenarios():
    S = []
    rnd = random.Random(7)

    def add(name, params=None, changes=None, sr=44100, blocks=120, block=512, channels=2, buffer=None):
        S.append(dict(name=name, sample_rate=sr, block=block, blocks=blocks, channels=channels,
                      buffer=buffer or block, params=params or {}, changes=changes or {}))

    add('flat')
    add('flat-mono', channels=1)
    add('random-faders', {'faders': [rnd.randrange(0, 266) for _ in range(32)]})
    add('all-zero', {'faders': [0] * 32})
    add('all-max', {'faders': [265] * 32})
    add('alternating', {'faders': curve(lambda k: 0 if k % 2 else 265)})
    add('notch-one', {'faders': curve(lambda k: 0 if k == 12 else 135)})
    add('smile', {'faders': curve(lambda k: 135 + 3.5 * (k - 15.5) ** 2 / 2)})
    add('ramp', {'faders': curve(lambda k: 8 * k)})
    add('breakpoints', {'faders': [0, 1, 2, 3, 4, 15, 16, 35, 55, 74, 75, 95, 115, 134, 135, 136,
                                   150, 165, 180, 195, 210, 225, 240, 254, 255, 256, 263, 264, 265, 100, 50, 10]})
    add('widest', {'low': 5, 'high': 412, 'faders': curve(lambda k: 60 + 6 * k)})
    add('narrow-low', {'low': 5, 'high': 175})
    add('upper-log', {'low': 250, 'high': 400, 'faders': curve(lambda k: 265 - 8 * k)})
    add('lo-above-352', {'low': 200, 'high': 412})
    add('needles-close', {'low': 300, 'high': 310})
    add('needles-crossed', {'low': 380, 'high': 200})
    add('push-high', {'low': 100, 'high': 300}, changes={'10': {'low': 290}, '30': {'low': 350}})
    add('push-low', {'low': 260, 'high': 400}, changes={'10': {'high': 270}, '40': {'high': 180}})
    add('automation', changes={
        '5': {'faders': curve(lambda k: 265 if k == 3 else 135)},
        '15': {'low': 120, 'high': 330},
        '25': {'faders': [rnd.randrange(0, 266) for _ in range(32)]},
        '26': {'faders': [rnd.randrange(0, 266) for _ in range(32)]},
        '40': {'low': 20, 'faders': curve(lambda k: 135 - 4 * k)},
        '70': {'high': 412, 'low': 300}})
    add('block-64', {'faders': curve(lambda k: 100 + 5 * k)}, block=64, blocks=600)
    add('block-1024', {'faders': curve(lambda k: 200 - 5 * k)}, block=1024, blocks=60)
    add('block-3000', {'faders': curve(lambda k: 0 if k > 20 else 180)}, block=3000, blocks=25)
    add('block-4096', {'low': 60}, block=4096, blocks=20)
    add('block-small-buffer-big', {'low': 80}, block=256, buffer=1024, blocks=240)
    add('long-run', {'faders': [rnd.randrange(60, 220) for _ in range(32)]}, blocks=1200)
    add('sr-48000', {'faders': curve(lambda k: 135 + (40 if k % 3 == 0 else -40))}, sr=48000)
    add('sr-96000', {'low': 30, 'high': 380, 'faders': curve(lambda k: 250 - 7 * k)}, sr=96000)
    add('sr-22050', {'faders': curve(lambda k: 30 + 7 * k)}, sr=22050)
    return S


def main():
    binary, runner, work = sys.argv[1:4]
    filt = sys.argv[4] if len(sys.argv) > 4 else ''
    os.makedirs(work, exist_ok=True)
    emulator = os.path.join(HERE, 'emulate_periscope.py')
    ok = fail = 0
    for sc in scenarios():
        if filt not in sc['name']:
            continue
        inp = os.path.join(work, f'input-{sc["sample_rate"]}.f32')
        if not os.path.exists(inp):
            make_input(inp, sc['sample_rate'])
        sc['input'] = inp
        js = os.path.join(work, sc['name'] + '.json')
        json.dump(sc, open(js, 'w'))
        eo, ro = js[:-5] + '.orig.f32', js[:-5] + '.rust.f32'
        t0 = time.time()
        run([sys.executable, emulator, binary, js, eo])
        t1 = time.time()
        run([runner, '--scenario', js, ro])
        same, why = compare(eo, ro)
        frames = sc['blocks'] * sc['block']
        print(f'{sc["name"]:<24} {frames:>8} frames  {"IDENTICAL" if same else "DIFFERENT"}  '
              f'(emulated in {t1 - t0:.1f}s) {why}', flush=True)
        ok += same
        fail += not same
    print(f'\n{ok} identical, {fail} different')
    sys.exit(1 if fail else 0)


if __name__ == '__main__':
    main()
