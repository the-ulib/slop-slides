"""Small offline CLI screen; not a production or minimum-hardware benchmark."""
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
parser.add_argument('models', type=Path)
parser.add_argument('output', type=Path)
parser.add_argument('--case', action='append', choices=['int8-en', 'bf16-en', 'int4-en', 'int8-de', 'bf16-de', 'long-en', 'bf16-long-en', 'clone-en', 'bf16-clone-en', 'bf16-clone-de'])
parser.add_argument('--presenter', type=Path, help='Saved .qvoice required for clone cases')
args = parser.parse_args()
if args.case and any('clone-' in name for name in args.case) and not args.presenter:
    parser.error('clone cases require --presenter')
args.output.mkdir(parents=True, exist_ok=True)
EN = 'Good work needs space. This presentation shows three simple ways to reduce interruptions, protect your attention, and make room for better ideas.'
DE = 'Gute Arbeit braucht Ruhe. In dieser Präsentation zeigen wir drei einfache Wege, Unterbrechungen zu reduzieren, die eigene Aufmerksamkeit zu schützen und Raum für bessere Ideen zu schaffen.'
LONG = EN + ' Begin by choosing a single priority for the morning. Turn off notifications and close the applications you do not need. Tell your colleagues when you will be available again. After a focused session, take a short break away from the screen. These changes do not require a perfect schedule. Try one small experiment this week, observe what helps, and adjust your routine. The goal is to create enough quiet time to think clearly and finish work that matters.'
CLONE = 'Tomorrow morning, protect one quiet hour for the task that matters most. A saved presenter lets you narrate another presentation without recording your voice again.'
cases = [('int8-en', EN, 'English', ['--int8']), ('bf16-en', EN, 'English', []), ('int4-en', EN, 'English', ['--int4']), ('int8-de', DE, 'German', ['--int8']), ('bf16-de', DE, 'German', []), ('long-en', LONG, 'English', ['--int8']), ('bf16-long-en', LONG, 'English', []), ('clone-en', CLONE, 'English', ['--int8']), ('bf16-clone-en', CLONE, 'English', []), ('bf16-clone-de', DE, 'German', [])]
results = []
for name, text, language, precision in cases:
    if (args.case and name not in args.case) or (not args.case and name not in ['int8-en', 'bf16-en', 'int4-en', 'int8-de']):
        continue
    output = args.output / (name + '.wav')
    max_duration = 90 if name.endswith('long-en') else 45
    is_clone = 'clone-' in name
    model = 'base' if is_clone else 'cv'
    voice = ['--load-voice', str(args.presenter)] if is_clone else ['-s', 'ryan']
    cmd = [str(args.engine), '-d', str(args.models / model), '--text', text, '-l', language, '--seed', '42', '--max-duration', str(max_duration), '-o', str(output)] + voice + precision
    env = os.environ.copy()
    controlled = {'QWEN_DISPATCH_MAP': '1', 'QWEN_SHAPE_CENSUS': '1', 'QWEN_CENSUS_JSON': str(args.output / (name + '-census.json')), 'QWEN_DISPATCH_JSON': str(args.output / (name + '-dispatch.json'))}
    for key, value in env.items():
        if key.startswith('QWEN_'):
            controlled.setdefault(key, value)
    env.update(controlled)
    print('START', name, flush=True)
    start = time.monotonic()
    with (args.output / (name + '.log')).open('w') as log:
        process = subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT, env=env)
        while True:
            waited, status, usage = os.wait4(process.pid, os.WNOHANG)
            if waited:
                process.returncode = os.waitstatus_to_exitcode(status)
                break
            if time.monotonic() - start > 180:
                process.kill()
                _, status, usage = os.wait4(process.pid, 0)
                process.returncode = os.waitstatus_to_exitcode(status)
                raise RuntimeError(name + ' exceeded 180 seconds')
            time.sleep(0.1)
    elapsed = time.monotonic() - start
    if process.returncode:
        raise RuntimeError(name + ' failed; see log')
    with wave.open(str(output), 'rb') as wav:
        assert wav.getnchannels() == 1 and wav.getsampwidth() == 2 and wav.getframerate() == 24000
        duration = wav.getnframes() / wav.getframerate()
        assert 0 < duration < max_duration
    row = {'case': name, 'wall_seconds': round(elapsed, 3), 'audio_seconds': duration, 'total_rtf': round(elapsed / duration, 3), 'peak_rss_bytes': usage.ru_maxrss if sys.platform == 'darwin' else usage.ru_maxrss * 1024, 'command': cmd, 'controlled_environment': controlled}
    results.append(row)
    (args.output / 'measurements.json').write_text(json.dumps({'classification': 'DIAGNOSTIC CLI screen, per-process model loading, warm filesystem cache; instrumented; not production-qualified', 'engine_sha256': hashlib.sha256(args.engine.read_bytes()).hexdigest(), 'results': results}, indent=2))
    print('DONE', json.dumps(row), flush=True)
