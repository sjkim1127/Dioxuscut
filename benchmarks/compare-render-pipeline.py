#!/usr/bin/env python3
"""Alternate immutable native binaries; verify raster bytes and decoded video frames."""
import argparse
import hashlib
from datetime import datetime, timezone
import json
import platform
import statistics
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser()
parser.add_argument('--before', type=Path, required=True)
parser.add_argument('--after', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--repeats', type=int, default=5)
parser.add_argument('--allow-busy', action='store_true', help='Record compiler-contended timings, marked as confounded')
args = parser.parse_args()
if args.repeats < 3:
    parser.error('use at least three measured repetitions')
def compiler_count():
    commands = subprocess.check_output(['ps', '-A', '-o', 'comm='], text=True).splitlines()
    return sum(Path(command.strip()).name in {'rustc', 'clippy-driver'} for command in commands)

if compiler_count() and not args.allow_busy:
    raise SystemExit('Compiler activity detected; finish competing builds before timing')
compiler_observations = []
folders = {'before': args.before.resolve(), 'after': args.after.resolve()}
workloads = [
    ('raster_720p', 'raster_bench', ['1280', '720', 'rects']),
    ('raster_1080p', 'raster_bench', ['1920', '1080', 'rects']),
    ('raster_4k', 'raster_bench', ['3840', '2160', 'rects']),
    ('raster_effects_1080p', 'raster_bench', ['1920', '1080', 'effects']),
    ('export_720p', 'render_bench', []),
    ('export_effects_1080p', 'cyberpunk_bench', []),
]
results = []
for name, binary, extra in workloads:
    export = not extra
    samples = {'before': [], 'after': []}
    pixels = {}
    for repetition in range(-1, args.repeats):
        for engine in (['before', 'after'] if repetition % 2 == 0 else ['after', 'before']):
            folder = folders[engine]
            command_args = [str(folder / f'{name}.mp4')] if export else extra
            compiler_observations.append(compiler_count())
            result = json.loads(subprocess.check_output([str(folder / binary), *command_args], cwd=ROOT, text=True))
            compiler_observations.append(compiler_count())
            if not export:
                if pixels and next(iter(pixels.values())) != result['hashes']:
                    raise RuntimeError(f'{name}: raster bytes changed')
                pixels[engine] = result['hashes']
            if repetition >= 0:
                samples[engine].append(result['render_ms'] if export else result['ms_per_frame'])
    if export:
        hashes = {}
        for engine in folders:
            decoded = subprocess.check_output(['ffmpeg', '-v', 'error', '-i', str(folders[engine] / f'{name}.mp4'), '-f', 'framemd5', '-'], text=True)
            hashes[engine] = [row for row in decoded.splitlines() if row and not row.startswith('#')]
        if hashes['before'] != hashes['after'] or len(hashes['before']) != 180:
            raise RuntimeError(f'{name}: decoded frame bytes/order/count changed')
        validation = {'identical_decoded_frames': 180}
    else:
        validation = {'identical_rgba_frames': 24, 'hashes': pixels['before']}
    medians = {engine: statistics.median(values) for engine, values in samples.items()}
    result = {'workload': name, 'unit': 'ms/export' if export else 'ms/frame', 'samples': samples,
              'median': medians, 'speedup': medians['before'] / medians['after'], 'validation': validation}
    results.append(result)
    print(f"{name:24} {medians['before']:9.3f} -> {medians['after']:9.3f} {result['unit']} ({result['speedup']:.3f}x)", flush=True)
report = {'recorded_at_utc': datetime.now(timezone.utc).isoformat(),
          'compiler_observations': compiler_observations, 'timing_confounded': any(compiler_observations),
          'platform': platform.platform(), 'repeats': args.repeats, 'warmup_runs': 1,
          'scope': 'same-machine native before/after, interleaved; excludes builds, startup, and validation',
          'ffmpeg': subprocess.check_output(['ffmpeg', '-version'], text=True).splitlines()[0],
          'binaries_sha256': {engine: {binary: hashlib.sha256((folder / binary).read_bytes()).hexdigest()
                                      for binary in ['raster_bench', 'render_bench', 'cyberpunk_bench']}
                              for engine, folder in folders.items()}, 'results': results}
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(report, indent=2) + '\n')

if any(compiler_observations) and not args.allow_busy:
    raise SystemExit('Competing compilation occurred during measurement; report saved as timing_confounded')
