"""Offline official Qwen speech reference; development-only, not a shipped worker."""
import argparse
import hashlib
import importlib.metadata
import json
from pathlib import Path
import time

parser = argparse.ArgumentParser()
parser.add_argument('model', type=Path)
parser.add_argument('output', type=Path)
parser.add_argument('--speaker', default='Ryan')
parser.add_argument('--language', default='English')
parser.add_argument('--seed', type=int, default=42)
parser.add_argument('--ref-audio', type=Path, help='Local reference WAV; requires Base pack and --ref-text')
parser.add_argument('--ref-text', help='Exact spoken reference words for ICL cloning')
parser.add_argument('--text', default='Good work needs space. This presentation shows three simple ways to reduce interruptions, protect your attention, and make room for better ideas.')
args = parser.parse_args()
if not (args.model / 'model.safetensors').is_file():
    parser.error('a complete local model pack is required')
if args.output.exists():
    parser.error('output already exists')
if bool(args.ref_audio) != bool(args.ref_text):
    parser.error('--ref-audio and --ref-text must be supplied together')
if args.ref_audio and not args.ref_audio.is_file():
    parser.error('local reference audio does not exist')

import numpy as np
import soundfile as sf
import torch
from qwen_tts import Qwen3TTSModel

torch.set_num_threads(4)
torch.manual_seed(args.seed)
np.random.seed(args.seed)
args.output.parent.mkdir(parents=True, exist_ok=True)
start = time.monotonic()
print('Loading official local model: CPU float32, eager attention, four threads', flush=True)
model = Qwen3TTSModel.from_pretrained(str(args.model), device_map='cpu', dtype=torch.float32, attn_implementation='eager', local_files_only=True)
loaded = time.monotonic()
print('Generating', args.speaker, args.language, flush=True)
settings = dict(do_sample=True, top_k=50, top_p=1.0, temperature=0.9, repetition_penalty=1.05, subtalker_dosample=True, subtalker_top_k=50, subtalker_top_p=1.0, subtalker_temperature=0.9, max_new_tokens=600)
if args.ref_audio:
    wavs, rate = model.generate_voice_clone(text=args.text, language=args.language, ref_audio=str(args.ref_audio), ref_text=args.ref_text, x_vector_only_mode=False, **settings)
else:
    wavs, rate = model.generate_custom_voice(text=args.text, speaker=args.speaker, language=args.language, **settings)
elapsed = time.monotonic()
audio = np.asarray(wavs[0])
assert rate == 24000 and audio.ndim == 1 and audio.size > 0
assert np.isfinite(audio).all() and np.max(np.abs(audio)) > 0
assert audio.size / rate < 48, 'output reached or exceeded the generation bound'
sf.write(str(args.output), audio, rate, subtype='PCM_16')
record = {'classification': 'Official reference quality sample; not a throughput benchmark or numerical-parity test', 'packages': {n: importlib.metadata.version(n) for n in ['qwen-tts', 'torch', 'transformers', 'numpy', 'soundfile']}, 'device': 'cpu', 'dtype': 'float32', 'attention': 'eager', 'threads': 4, 'speaker': args.speaker, 'language': args.language, 'text': args.text, 'seed': args.seed, 'settings': settings, 'load_seconds': round(loaded-start, 3), 'generation_seconds': round(elapsed-loaded, 3), 'sample_rate': rate, 'sample_count': int(audio.size), 'audio_seconds': audio.size/rate, 'wav_sha256': hashlib.sha256(args.output.read_bytes()).hexdigest()}
record['mode'] = 'ICL clone' if args.ref_audio else 'CustomVoice'
if args.ref_audio:
    record['speaker'] = None
    record['reference_sha256'] = hashlib.sha256(args.ref_audio.read_bytes()).hexdigest()
    record['reference_text'] = args.ref_text
args.output.with_suffix('.json').write_text(json.dumps(record, indent=2, ensure_ascii=False)+'\n')
print(json.dumps(record, ensure_ascii=False), flush=True)
