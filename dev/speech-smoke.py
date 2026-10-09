"""Explicit local-model smoke for the bundled worker; no downloads or Python ML stack."""
import argparse
import hashlib
import json
from pathlib import Path
import queue
import subprocess
import threading
import time
import wave

p = argparse.ArgumentParser()
p.add_argument('binary', type=Path)
p.add_argument('model', type=Path)
p.add_argument('output', type=Path)
a = p.parse_args()
if a.output.exists(): p.error('output must be a new directory')
a.output.mkdir(parents=True)
subprocess.run([str(a.binary), '--self-test'], check=True, timeout=10)
class Worker:
    def __init__(self):
        self.process = subprocess.Popen([str(a.binary), str(a.model)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=(a.output/'worker.log').open('ab'))
        self.events = queue.Queue()
        def read():
            for raw in self.process.stdout:
                if raw.startswith(b'SLOPSPEECH '): self.events.put(json.loads(raw[len(b'SLOPSPEECH '):]))
            self.events.put({'type':'eof'})
        threading.Thread(target=read, daemon=True).start()
        assert self.events.get(timeout=180)['type'] == 'ready'
    def send(self, name, text, language, pace):
        request = dict(text=text, speaker='ryan', language=language, pace=str(pace), output=str(a.output/f'{name}.wav'))
        self.process.stdin.write((json.dumps(request)+'\n').encode()); self.process.stdin.flush()
    def finish(self, name):
        while True:
            event = self.events.get(timeout=180)
            if event['type'] == 'progress': continue
            assert event['type'] == 'done', event
            with wave.open(str(a.output/f'{name}.wav'), 'rb') as wav:
                assert (wav.getframerate(),wav.getnchannels(),wav.getsampwidth()) == (24000,1,2)
                assert wav.getnframes() == event['samples']
                assert 0 < wav.getnframes() < 50*24000
            return event['samples'] / 24000
    def close(self):
        self.process.stdin.close(); assert self.process.wait(timeout=10) == 0
english='Good work needs space. This presentation shows three simple ways to reduce interruptions, protect your attention, and make room for better ideas.'
german='Gute Arbeit braucht Ruhe. In dieser Präsentation zeigen wir drei einfache Wege, Unterbrechungen zu reduzieren, die eigene Aufmerksamkeit zu schützen und Raum für bessere Ideen zu schaffen.'
start=time.monotonic(); worker=Worker(); results={}
for name,text,language,pace in [('english',english,'English',1),('german',german,'German',1.1),('english-paced',english,'English',1.1)]:
    worker.send(name,text,language,pace); results[name]=worker.finish(name)
assert results['english-paced'] < results['english'] * 0.96
worker.send('cancelled',german,'German',1.1)
while worker.events.get(timeout=180)['type'] != 'progress': pass
cancel_start=time.monotonic(); worker.process.kill(); worker.process.wait(timeout=5)
assert not (a.output/'cancelled.wav').exists()
results['hard_cancel_seconds']=time.monotonic()-cancel_start
retry=Worker(); retry.send('retry',english,'English',1); results['retry']=retry.finish('retry'); retry.close()
assert (a.output/'english.wav').read_bytes() == (a.output/'retry.wav').read_bytes()
results.update(wall_seconds=time.monotonic()-start, wav_sha256={f.name:hashlib.sha256(f.read_bytes()).hexdigest() for f in a.output.glob('*.wav')})
(a.output/'results.json').write_text(json.dumps(results,indent=2)+'\n')
print(json.dumps(results,indent=2))
