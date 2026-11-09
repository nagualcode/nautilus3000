#!/usr/bin/env python3
"""Differential test: the original Deep Phase Nine (emulated) against the Rust
engine, bit for bit.

    differential_test.py ORIGINAL_I386_BINARY path/to/dpn-render WORKDIR [name-filter]
"""
import json
import math
import os
import random
import struct
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))


def make_input(path, sr, seconds=6.0, seed=3):
    """Stereo test signal: noise plus tones, with near-silent passages so the
    feedback paths decay into the denormal range."""
    n = int(sr * seconds)
    rnd = random.Random(seed)
    v = []
    for i in range(n):
        t = i / sr
        l = 0.25 * (rnd.random() - 0.5) + 0.2 * math.sin(2 * math.pi * 220 * t) \
            + 0.1 * math.sin(2 * math.pi * 1730 * t + math.sin(t))
        r = 0.25 * (rnd.random() - 0.5) + 0.2 * math.sin(2 * math.pi * 330 * t)
        if int(t * 2) % 4 == 3:
            l *= 0.001
            r *= 0.001
        v += [l, r]
    with open(path, 'wb') as f:
        f.write(struct.pack(f'<{len(v)}f', *v))


def scenarios():
    S = []

    def add(name, params=None, changes=None, sr=44100, blocks=200, block=512, layout='stereo', transport=None):
        S.append(dict(name=name, sample_rate=sr, block=block, blocks=blocks, layout=layout,
                      params=params or {}, changes=changes or {}, transport=transport))

    add('defaults')
    for shape in (1, 2, 3, 4, 5):
        add(f'lfo-{shape}', {'lfo_choice': shape, 'lfo_speed': 45, 'stereo_offset': 60})
    for n in (1, 2, 6, 12, 24):
        add(f'notches-{n}', {'notches': n, 'lfo_speed': 35})
    add('feedback-max', {'feedback': 0.95, 'notches': 8, 'spread': 0})
    add('feedback-min', {'feedback': -0.95, 'notches': 8, 'spread': 0})
    add('spread-max', {'spread': 100, 'notches': 12})
    add('range-extreme', {'low': 0.01, 'high': 20000, 'lfo_speed': 50})
    add('range-inverted', {'low': 2000, 'high': 200})
    add('wet-dry', {'wet_dry': 0.37, 'feedback': -0.5})
    add('offset-180', {'stereo_offset': 180, 'lfo_choice': 2, 'lfo_speed': 40})
    add('offset-neg', {'stereo_offset': -77, 'lfo_choice': 3, 'lfo_speed': 55})
    add('lfo-fastest', {'lfo_speed': 70, 'lfo_choice': 1})
    add('sh-fastest', {'lfo_speed': 70, 'lfo_choice': 3})
    add('oneshot-sine-down', {'lfo_choice': 4, 'oneshot_dir': -1, 'oneshot_end': 0.2, 'lfo_speed': 50})
    add('oneshot-sine-up', {'lfo_choice': 4, 'oneshot_dir': 1, 'oneshot_end': 0.8, 'lfo_speed': 50})
    add('oneshot-tri-up', {'lfo_choice': 5, 'oneshot_dir': 1, 'oneshot_end': 0.6, 'lfo_speed': 55})
    add('oneshot-dir-0', {'lfo_choice': 5, 'oneshot_dir': 0, 'lfo_speed': 55})
    add('automation', changes={
        '20': {'lfo_choice': 3}, '40': {'notches': 17, 'feedback': -0.3}, '60': {'lfo_choice': 4},
        '80': {'lfo_choice': 2, 'lfo_speed': 60}, '100': {'spread': 90, 'low': 30},
        '120': {'lfo_choice': 5, 'oneshot_dir': 1}, '150': {'lfo_choice': 1, 'stereo_offset': -120}})
    play = {'bpm': 120.0, 'playing': True, 'start_ppq': 0.37}
    for shape in (1, 2, 3):
        add(f'sync-{shape}', {'sync': 1, 'lfo_choice': shape, 'lfo_speed': 50}, transport=play)
    add('sync-sh-speed-changes', {'sync': 1, 'lfo_choice': 3}, transport=play,
        changes={'30': {'lfo_speed': 40}, '70': {'lfo_speed': 55}, '120': {'lfo_speed': 20}})
    add('sync-97bpm', {'sync': 1, 'lfo_speed': 70}, transport={'bpm': 97.0, 'playing': True, 'start_ppq': 5.9})
    add('layout-mono', {'lfo_choice': 3, 'notches': 9}, layout='mono')
    add('layout-mono-fb', {'feedback': 0.95, 'lfo_choice': 1}, layout='mono')
    add('layout-mono-to-stereo', {'stereo_offset': 90}, layout='mono_to_stereo')
    add('block-64', {'lfo_speed': 50}, block=64, blocks=1500)
    add('block-1024', {'lfo_choice': 2}, block=1024, blocks=100)
    add('long-run', {'lfo_speed': 30, 'notches': 6}, blocks=2000)
    add('sr-48000', {'lfo_choice': 3}, sr=48000)
    add('sr-96000', {'notches': 24, 'feedback': 0.9}, sr=96000, blocks=300)
    return S


def run(cmd):
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f'{" ".join(cmd)}\n{r.stdout}\n{r.stderr}')


def compare(a, b):
    x, y = open(a, 'rb').read(), open(b, 'rb').read()
    if x == y:
        return True, ''
    n = min(len(x), len(y)) // 4
    xa, ya = struct.unpack(f'<{n}f', x[:4 * n]), struct.unpack(f'<{n}f', y[:4 * n])
    d = [i for i in range(n) if xa[i] != ya[i]]
    if not d:
        return False, f'length {len(x)} vs {len(y)}'
    peak = max(map(abs, xa)) or 1.0
    md = max(abs(xa[i] - ya[i]) for i in d)
    return False, f'{len(d)} values differ, first at value {d[0]}, max |diff| {md:.3e} ({md / peak:.1e} of peak)'


def main():
    binary, runner, work = sys.argv[1:4]
    filt = sys.argv[4] if len(sys.argv) > 4 else ''
    os.makedirs(work, exist_ok=True)
    emulator = os.path.join(HERE, 'emulate_deepphasenine.py')
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
