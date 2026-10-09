"""Build a locally signed diagnostic bundle; never modifies the source model pack."""
import argparse
from pathlib import Path
import os
import plistlib
import shutil
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument('worker', type=Path)
parser.add_argument('model', type=Path)
parser.add_argument('output', type=Path)
parser.add_argument('--identity', default='-', help='codesign identity; default is ad-hoc')
parser.add_argument('--identifier', default='dev.slopslide.feasibility.worker20261009')
args = parser.parse_args()
if args.output.exists():
    parser.error('choose a new output directory')
if not args.worker.is_file() or not (args.model / 'model.safetensors').is_file():
    parser.error('existing worker and complete local model pack required')
root = args.output.resolve()
contents = root / 'SlopSlideSpeechProbe.app' / 'Contents'
for part in ['MacOS', 'Helpers', 'Resources']:
    (contents / part).mkdir(parents=True, exist_ok=True)
source = Path(__file__).with_name('sandbox_worker.swift').resolve()
subprocess.run(['xcrun', 'swiftc', '-parse-as-library', '-module-cache-path', str(root / 'swift-cache'), str(source), '-o', str(contents / 'MacOS' / 'speech-probe')], check=True)
shutil.copyfile(args.worker, contents / 'Helpers' / 'qwen_tts')
(contents / 'Helpers' / 'qwen_tts').chmod(0o755)
# Hardlinks avoid another multi-GB copy on the same volume. No code edits the weights.
def resource_copy(src, dst):
    try:
        os.link(src, dst)
    except OSError:
        shutil.copyfile(src, dst)
    return dst
shutil.copytree(args.model, contents / 'Resources' / 'cv', copy_function=resource_copy)
identifier = args.identifier
(contents / 'Info.plist').write_bytes(plistlib.dumps({'CFBundleIdentifier': identifier, 'CFBundleName': 'SlopSlideSpeechProbe', 'CFBundleExecutable': 'speech-probe', 'CFBundlePackageType': 'APPL', 'CFBundleVersion': '1', 'CFBundleShortVersionString': '0.1', 'LSUIElement': True}))
(root / 'parent.entitlements').write_bytes(plistlib.dumps({'com.apple.security.app-sandbox': True}))
(root / 'worker.entitlements').write_bytes(plistlib.dumps({'com.apple.security.app-sandbox': True, 'com.apple.security.inherit': True}))
(root / 'outside-fixture.txt').write_text('Harmless sandbox negative-control fixture.\n')
for command in [
    ['codesign', '--force', '--sign', args.identity, '--identifier', identifier+'.worker', '--entitlements', str(root/'worker.entitlements'), str(contents/'Helpers'/'qwen_tts')],
    ['codesign', '--force', '--sign', args.identity, '--entitlements', str(root/'parent.entitlements'), str(contents.parent)],
    ['codesign', '--verify', '--deep', '--strict', str(contents.parent)],
]:
    subprocess.run(command, check=True)
print('Signed diagnostic bundle:', contents.parent)
print('Run:', contents/'MacOS'/'speech-probe', root/'outside-fixture.txt')
