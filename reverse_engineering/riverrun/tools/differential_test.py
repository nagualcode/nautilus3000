#!/usr/bin/env python3
"""Differential test: the original Riverrun (emulated) against the Rust engine.

    differential_test.py ORIGINAL_I386_BINARY path/to/riverrun-render WORKDIR [name-filter]

For every scenario below both implementations render the same blocks from the
same input; the outputs must be identical bit for bit.
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


def make_snippet(path, sr, seed=7):
    """A test "utterance": two gliding partials with a slow swell and noise
    bursts every few hundred ms, exactly one snapshot (4.65 s) long."""
    n = int(sr * 4.65)
    rnd = random.Random(seed)
    v = []
    for i in range(n):
        t = i / sr
        env = 0.5 + 0.5 * math.sin(t * 2.1)
        s = 0.4 * math.sin(2 * math.pi * 196 * t + 2 * math.sin(2 * math.pi * 3 * t))
        s += 0.2 * math.sin(2 * math.pi * 587 * t)
        s = s * env + (0.3 * (rnd.random() - 0.5) if int(t * 4) % 3 == 0 else 0.0)
        v.append(s)
    with open(path, 'wb') as f:
        f.write(struct.pack(f'<{n}f', *v))


def scenarios():
    S = []

    def add(name, params=None, changes=None, sr=44100, blocks=300, block=512, live=False,
            transport=None):
        S.append(dict(name=name, sample_rate=sr, block=block, blocks=blocks,
                      params=params or {}, changes=changes or {}, live_input=live,
                      transport=transport))

    add('defaults')
    for env in (1, 2, 3, 4):
        add(f'envelope-{env}', {'envelope': env, 'length': 230})
    for grid in range(1, 24):
        add(f'grid-{grid}', {'grid': grid, 'pitch': 700, 'random': 95}, blocks=150)
    add('pitch-low', {'pitch': 0, 'length': 286})
    add('pitch-high', {'pitch': 1000, 'random': 100, 'grid': 2})
    add('fine-tune', {'fine': 0.8, 'pitch': 333})
    add('walk-back', {'walk': -2.5})
    add('walk-stop', {'walk': 0.0, 'glass': 460})
    add('walk-fast-narrow', {'walk': 5.0, 'needle_l': 200, 'needle_r': 230}, blocks=600)
    add('long-grains-at-end', {'length': 286, 'needle_r': 472, 'walk': 1.0}, blocks=500)
    add('long-grains-backwards', {'length': 286, 'needle_r': 472, 'walk': -0.3, 'glass': 470})
    add('dense', {'speed': 286, 'length': 200}, blocks=120)
    add('sparse', {'speed': 1, 'speed_random': 0}, blocks=400)
    add('jitter-max', {'speed_random': 286, 'speed': 200})
    add('mono', {'mono_stereo': 0})
    add('half-stereo', {'mono_stereo': 40})
    add('automation', changes={
        '20': {'envelope': 3}, '40': {'pitch': 800, 'grid': 11, 'random': 60},
        '80': {'envelope': 4, 'speed': 250}, '120': {'walk': -1}, '160': {'needle_l': 300},
        '200': {'mono_stereo': 10, 'length': 20}, '250': {'envelope': 1, 'fine': 1.4}})
    play120 = {'bpm': 120.0, 'playing': True, 'start_ppq': 0.37}
    for div in (0, 4, 7, 13):
        add(f'rhythmic-div-{div}', {'rhythmic': 1, 'speed': div * 21.142857 + 3},
            transport=play120)
    add('rhythmic-93bpm-jitter', {'rhythmic': 1, 'speed': 150, 'speed_random': 120},
        transport={'bpm': 93.0, 'playing': True, 'start_ppq': 3.9})
    add('rhythmic-stopped', {'rhythmic': 1, 'speed': 120},
        transport={'bpm': 120.0, 'playing': False})
    add('rhythmic-to-flowing', {'rhythmic': 1}, changes={'60': {'rhythmic': 0}},
        transport=play120)
    add('rhythmic-speed-changes', {'rhythmic': 1}, transport=play120,
        changes={'50': {'speed': 200}, '120': {'speed': 60}, '200': {'speed': 286}})
    add('record-live', {'record': 1}, live=True, blocks=600)
    add('record-then-freeze', {'record': 1}, changes={'300': {'record': 0}}, live=True, blocks=600)
    add('host-block-64', block=64, blocks=2000)
    add('host-block-1024', {'speed': 220}, block=1024, blocks=200)
    add('long-run-bus-wrap', {'speed': 180, 'walk': 0.05}, blocks=2500)
    add('sr-48000', sr=48000)
    add('sr-96000', {'length': 286, 'speed_random': 286}, sr=96000, blocks=400)
    return S


def run(cmd):
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f'{" ".join(cmd)}\n{r.stdout}\n{r.stderr}')
    return r.stdout.strip()


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
    return False, f'{len(d)} values differ, first at frame {d[0] // 2}, max |diff| {md:.3e} ({md / peak:.1e} of peak)'


def main():
    binary, runner, work = sys.argv[1:4]
    filt = sys.argv[4] if len(sys.argv) > 4 else ''
    os.makedirs(work, exist_ok=True)
    emulator = os.path.join(HERE, 'emulate_riverrun.py')
    py = sys.executable
    ok = fail = 0
    for sc in scenarios():
        if filt not in sc['name']:
            continue
        snip = os.path.join(work, f'snippet-{sc["sample_rate"]}.f32')
        if not os.path.exists(snip):
            make_snippet(snip, sc['sample_rate'])
        sc['snippet'] = snip
        js = os.path.join(work, sc['name'] + '.json')
        json.dump(sc, open(js, 'w'))
        eo, ro = js[:-5] + '.orig.f32', js[:-5] + '.rust.f32'
        t0 = time.time()
        run([py, emulator, binary, js, eo])
        t1 = time.time()
        run([runner, '--scenario', js, ro])
        same, why = compare(eo, ro)
        frames = sc['blocks'] * sc['block']
        status = 'IDENTICAL' if same else 'DIFFERENT'
        print(f'{sc["name"]:<26} {frames:>8} frames  {status}  (emulated in {t1 - t0:.1f}s) {why}')
        ok += same
        fail += not same
    print(f'\n{ok} identical, {fail} different')
    sys.exit(1 if fail else 0)


if __name__ == '__main__':
    main()
