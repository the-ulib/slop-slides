"""Build the bundled macOS render/encode helper; users need no compiler."""
from pathlib import Path
import hashlib, platform, subprocess, sys
root = Path(__file__).resolve().parent.parent
if sys.platform != 'darwin':
    print('Narrated video encoding currently targets macOS.')
    sys.exit(0)
source = root / 'src-tauri/video-worker/main.swift'
out = root / 'src-tauri/video-runtime'
out.mkdir(exist_ok=True)
binary = out / 'slopslide-video'
stamp = out / 'build.txt'
identity = hashlib.sha256(source.read_bytes() + platform.machine().encode()).hexdigest()
if binary.exists() and stamp.exists() and stamp.read_text() == identity:
    print('Video helper is current.')
else:
    subprocess.run(['xcrun', 'swiftc', '-parse-as-library', '-O', '-module-cache-path', str(root / 'src-tauri/target/swift-module-cache'), '-target', f'{platform.machine()}-apple-macosx12.0', str(source), '-o', str(binary)], check=True)
    stamp.write_text(identity)
    print(f'Built {binary}')
