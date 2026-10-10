"""Build the pinned CPU speech helper for macOS. End users need no Python/compiler."""
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
RUNTIME = ROOT / 'src-tauri/speech-runtime'
REV = 'ef339be58a778b062e1c14382347964552eae007'
RUNTIME.mkdir(exist_ok=True)
if sys.platform != 'darwin':
    print('Native speech currently targets macOS; other platforms can edit scripts.')
    sys.exit(0)
engine = ROOT / 'src-tauri/target/speech-engine'
source = ROOT / 'src-tauri/speech-worker/worker.c'
sonic = source.parent / 'vendor/sonic'
inputs = {'engineRevision': REV, 'platform': platform.machine(), 'protocol': 1,
          'sources': {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
                      for p in [source, source.parent / 'profile.h', sonic / 'sonic.c', sonic / 'sonic.h']},
          'flags': '-O3 -ffast-math -march=armv8-a' if platform.machine() == 'arm64' else '-O3 -ffast-math -msse2'}
stamp = RUNTIME / 'build.json'
binary = RUNTIME / 'slopslide-speech'
if binary.is_file() and stamp.is_file() and json.loads(stamp.read_text()) == inputs:
    print('Speech helper is current.')
    sys.exit(0)
if not (engine / '.git').exists():
    engine.parent.mkdir(parents=True, exist_ok=True)
    local = os.getenv('SLOPSLIDE_QWEN_SOURCE')
    if local:
        subprocess.run(['git', 'clone', '--no-hardlinks', local, str(engine)], check=True)
    else:
        subprocess.run(['git', 'init', str(engine)], check=True)
        subprocess.run(['git', '-C', str(engine), 'fetch', '--depth', '1',
                        'https://github.com/gabriele-mastrapasqua/qwen3-tts.git', REV], check=True)
    subprocess.run(['git', '-C', str(engine), 'checkout', '--detach', REV], check=True)
assert subprocess.check_output(['git', '-C', str(engine), 'rev-parse', 'HEAD'], text=True).strip() == REV
assert not subprocess.check_output(['git', '-C', str(engine), 'status', '--porcelain', '--untracked-files=no'], text=True).strip(), 'engine checkout must be clean'
makefile = engine.parent / 'Makefile.speech-compat'
makefile.write_text((engine / 'Makefile').read_text().replace("'^#define __ARM_FEATURE_BF16 '", "'^\\#define __ARM_FEATURE_BF16 '"))
arch = '-march=armv8-a' if platform.machine() == 'arm64' else '-msse2'
subprocess.run(['make', '-f', str(makefile), 'blas', 'CC=clang', f'ARCH_FLAGS={arch}', '-j4'], cwd=engine, check=True)
objects = sorted(p for p in engine.glob('*.o') if p.name != 'main.o')
objects += sorted((engine / 'third_party/kleidiai').rglob('*.o'))
objects += [engine / 'vendor/lz4.o', engine / 'third_party/ingot/libingot.a']
assert not any(p.name in ['qwen_tts_metal.o', 'qwen_tts_cuda.o', 'qwen_tts_backend.o'] for p in objects), 'CPU objects required'
subprocess.run(['clang', '-O2', '-std=c11', '-Wall', '-Wextra', '-I', str(engine), '-I', str(sonic),
                str(source), str(sonic / 'sonic.c'), *map(str, objects), '-framework', 'Accelerate',
                '-lm', '-lpthread', '-o', str(binary)], check=True)
# Carry notices with the shipped runtime, including upstream embedded dependency terms.
shutil.copyfile(engine / 'LICENSE', RUNTIME / 'Qwen-C-LICENSE.txt')
shutil.copyfile(engine / 'third_party/ingot/LICENSE', RUNTIME / 'Ingot-LICENSE.txt')
shutil.copyfile(sonic / 'LICENSE', RUNTIME / 'Sonic-LICENSE.txt')
(RUNTIME / 'LZ4-NOTICE.txt').write_text((engine / 'vendor/lz4.c').read_text().split('*/', 1)[0] + '*/\n')
shutil.copyfile(sonic / 'LICENSE', RUNTIME / 'Qwen-Model-APACHE-2.0.txt')
stamp.write_text(json.dumps(inputs, indent=2) + '\n')
print(f'Built {binary}')
