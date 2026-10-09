"""Mac-only bounded resident-context diagnostic using the existing pinned CPU objects."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import wave

parser = argparse.ArgumentParser()
parser.add_argument('engine', type=Path)
parser.add_argument('model', type=Path)
parser.add_argument('output', type=Path)
args = parser.parse_args()
if sys.platform != 'darwin':
    parser.error('this diagnostic requires macOS')
if args.output.exists():
    parser.error('output must be a new directory')
if not (args.engine / 'qwen_tts.o').is_file() or not (args.model / 'model.safetensors').is_file():
    parser.error('built CPU engine objects and local CustomVoice pack required')
objects = sorted(p for p in args.engine.glob('*.o') if p.name != 'main.o')
objects += sorted((args.engine / 'third_party/kleidiai').rglob('*.o'))
objects += [args.engine / 'vendor/lz4.o', args.engine / 'third_party/ingot/libingot.a']
if not all(p.is_file() for p in objects):
    parser.error('complete CPU build required')
if any(p.name in ['qwen_tts_metal.o', 'qwen_tts_cuda.o', 'qwen_tts_backend.o'] for p in objects):
    parser.error('use the CPU-only build for this diagnostic')
args.output.mkdir(parents=True)
binary = args.output / 'resident-probe'
source = Path(__file__).with_suffix('.c').resolve()
subprocess.run(['clang', '-O2', '-std=c11', '-Wall', '-Wextra', '-I', str(args.engine), str(source), *map(str, objects), '-framework', 'Accelerate', '-lm', '-lpthread', '-o', str(binary)], check=True)
env = {k: v for k, v in os.environ.items() if not k.startswith('QWEN_')}
env.update(QWEN_NO_KLEIDI='1', QWEN_DISPATCH_MAP='1', QWEN_SHAPE_CENSUS='1', QWEN_CENSUS_JSON=str(args.output/'census.json'))
start = time.monotonic()
with (args.output/'run.log').open('w') as log:
    subprocess.run([str(binary), str(args.model), str(args.output)], env=env, stdout=log, stderr=subprocess.STDOUT, timeout=120, check=True)
elapsed = time.monotonic()-start
rows = [json.loads(line.removeprefix('PROBE ')) for line in (args.output/'run.log').read_text().splitlines() if line.startswith('PROBE ')]
assert len(rows) == 5 and all(row.get('valid', True) for row in rows)
assert rows[3]['cancelled'] and rows[3]['generated_frames'] <= 8
hashes = {}
for i in [0, 1, 3]:
    p = args.output/f'job-{i}.wav'
    with wave.open(str(p),'rb') as w:
        assert w.getframerate() == 24000 and w.getnchannels() == 1 and w.getsampwidth() == 2
        assert w.getnframes() == rows[i+1]['samples']
    hashes[p.name] = hashlib.sha256(p.read_bytes()).hexdigest()
assert not (args.output/'job-2.wav').exists()
assert hashes['job-0.wav'] == hashes['job-3.wav'], 'repeated job changed after language switch/cancellation'
record = {'classification': 'DIAGNOSTIC resident CPU API PoC, warm filesystem/shared M4 Pro host; not production or baseline-device qualification', 'upstream_revision': subprocess.check_output(['git','-C',str(args.engine),'rev-parse','HEAD'],text=True).strip(), 'upstream_dirty': subprocess.check_output(['git','-C',str(args.engine),'status','--porcelain'],text=True).strip(), 'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest(), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'objects': {str(p.relative_to(args.engine)): hashlib.sha256(p.read_bytes()).hexdigest() for p in objects}, 'command': ['<EVIDENCE>/resident-probe','<MODELS>/cv','<EVIDENCE>'], 'controlled_environment': {k:v.replace(str(args.output),'<EVIDENCE>') for k,v in env.items() if k.startswith('QWEN_')}, 'threads':4, 'precision':'BF16 mode, no Kleidi packing', 'host':'Apple M4 Pro, 48 GiB, macOS 26.6.2', 'wall_seconds':elapsed, 'events':rows, 'wav_sha256':hashes, 'repeat_after_language_switch_and_cancel':'byte-identical WAV', 'lifecycle':'process exited normally; timeout bounded at 120 seconds'}
(args.output/'measurements.json').write_text(json.dumps(record,indent=2)+'\n')
print(json.dumps({'events':rows,'repeat':'byte-identical','wall_seconds':round(elapsed,3)},indent=2))
