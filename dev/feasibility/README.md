# Standalone narration feasibility probes

These development probes support [the review](../../docs/narration-feasibility.md). They are not wired into Tauri and are not production implementations. No models, third-party executables or private presenter profiles are committed. Native probes require a Mac with Xcode; the timing harness uses Python 3.9+ and `wait4` (macOS/Linux). FFmpeg is a development verification tool only.

## Reproduce local speech

Use scratch directories, with `ENGINE` pointing at the C checkout, `MODELS` at a directory containing `cv/` and `base/`, and `EVIDENCE` at an empty output directory. Quote these variables when using them in shell commands.

- C source: `https://github.com/gabriele-mastrapasqua/qwen3-tts.git`, revision `ef339be58a778b062e1c14382347964552eae007`.
- CustomVoice: `Qwen/Qwen3-TTS-12Hz-0.6B-CustomVoice`, revision `85e237c12c027371202489a0ec509ded67b5e4b5`, downloaded to `cv/`.
- Base: `Qwen/Qwen3-TTS-12Hz-0.6B-Base`, revision `5d83992436eae1d760afd27aff78a71d676296fc`, downloaded to `base/`.

Download the complete official model packs, including `speech_tokenizer/`, configurations, vocabulary and merges. A development Hugging Face CLI supports `hf download MODEL_ID --revision REVISION --local-dir DIRECTORY`. Verify sizes/checksums against the pinned manifests; key weight hashes are recorded in the review. Initial downloads are several GB. Identical codec resources can be shared, but verify both manifests first.

Read the upstream instructions/licenses. On the tested Mac, build with `make blas CC=clang -j4`. Apple Make 3.81 failed parsing the upstream Makefile's `'^#define __ARM_FEATURE_BF16 '` shell expression. The tested compatibility copy escapes that hash as `'^\#define __ARM_FEATURE_BF16 '`, then builds with `make -f /absolute/path/Makefile.compat blas CC=clang -j4` from the engine directory. No C source change was made. Record your actual source revision, build command, overrides and executable hash.

```sh
"$ENGINE/qwen_tts" --caps
"$ENGINE/qwen_tts" --self-test
python3 dev/feasibility/benchmark.py "$ENGINE/qwen_tts" "$MODELS" "$EVIDENCE/screen"
QWEN_NO_KLEIDI=1 python3 dev/feasibility/benchmark.py "$ENGINE/qwen_tts" "$MODELS" "$EVIDENCE/lowmem" --case int8-en --case long-en
```

The harness writes one WAV/log/census and a measurement record per case. Environment variables prefixed `QWEN_` are recorded; use a controlled environment and do not put secrets in them. Its timing includes model loading; it does not measure a warm resident worker. It runs cases sequentially and checks successful exit, mono PCM16 at 24 kHz and bounded nonzero duration. Peak RSS covers the executable process, not the application or OS-wide memory pressure. Do not infer supported hardware from this one machine.

### Save and reload a presenter

This reproducible test uses the generated English sample as a synthetic reference. It tests persistence without recording a real person.

```sh
"$ENGINE/qwen_tts" -d "$MODELS/base" --ref-audio "$EVIDENCE/screen/int8-en.wav" --ref-text 'Good work needs space. This presentation shows three simple ways to reduce interruptions, protect your attention, and make room for better ideas.' --save-voice "$EVIDENCE/presenter.qvoice" --voice-name Feasibility --int8 -l English
QWEN_NO_KLEIDI=1 python3 dev/feasibility/benchmark.py "$ENGINE/qwen_tts" "$MODELS" "$EVIDENCE/clone" --case clone-en --presenter "$EVIDENCE/presenter.qvoice"
```

The second command starts a fresh engine process with only `--load-voice`; it does not pass the reference recording/transcript. The tested profile is approximately 16 MB. Do not commit real reference recordings or presenter profiles. Listen to generated output; successful CLI execution does not establish similarity or speech quality.

## Reproduce the native video

Run from the repository root. Set `EVIDENCE` to an existing writable scratch directory; output names must be unused. Xcode compilation may need a separate writable module cache in restricted environments.

```sh
xcrun swiftc -parse-as-library dev/feasibility/snapshot.swift -o "$EVIDENCE/snapshot"
xcrun swiftc -parse-as-library dev/feasibility/encode.swift -o "$EVIDENCE/encode"
"$EVIDENCE/snapshot" dev/feasibility/slide.html "$EVIDENCE/slide.png"
"$EVIDENCE/encode" "$EVIDENCE/slide.png" "$EVIDENCE/screen/int8-en.wav" "$EVIDENCE/narrated-slide.mp4"
ffprobe -v error -show_entries stream=codec_name,width,height,sample_rate,channels,duration -show_entries format=duration,size -of json "$EVIDENCE/narrated-slide.mp4"
ffmpeg -v error -i "$EVIDENCE/narrated-slide.mp4" -f null -
```

The snapshot optionally accepts an extra `attached` argument to attach the webview to an unshown window. The tested default renders without a window. Readiness and Retina normalization are intentionally visible in the probe. The encoder repeats one slide frame at 30 fps, then muxes narration using AVFoundation. These probes do not implement app cancellation, multi-slide timelines, atomic publication, resource freezing or a sandboxed worker. They require normal access to system WebKit and encoding services; the coding tool sandbox is a different boundary from an App Sandbox release build.

## Test cases and release gates

| Case | Expected result | Status in this review |
| --- | --- | --- |
| C built-in self-test | Zero failed cases | Passed |
| English/German stock synthesis | Valid bounded WAV, inspectable operation census | Passed |
| Low-memory short/long speech | Valid WAV; record full process peak RSS and wall time | Passed for the recorded host |
| Profile reload in separate process | New speech with no reference WAV/transcript arguments | Passed with a synthetic reference |
| Missing `--presenter` for clone case | Argument error before launching the engine | Passed |
| Hidden static HTML snapshot | Exactly 1920×1080 pixels, correct visual contents | Passed |
| Snapshot with missing HTML input | Nonzero exit; no PNG | Passed |
| Native encode/decode | H.264 1080p plus AAC; duration difference ≤1 frame; full decode without errors | Passed |
| Encode existing destination | Nonzero exit; existing output bytes unchanged | Passed |
| Encode wrong-size image | Nonzero exit; no destination | Passed |
| Integrated app on baseline Mac | App plus worker fits target with acceptable swap/throughput | Pending |
| Human-reference cloning | Recognizable, stable English/German clone in listening tests | German-reference likeness accepted; English usable with slight accepted accent; broader corpus/personal reference pending |
| Signed sandbox Tauri/worker | Model load, inference, profile storage, permissions and export succeed | Pending |
| Full deck with assets and edits | Frozen resources, accurate boundaries, cancellation and failures handled | Pending |
| Native Windows/Linux | Packaged inference and encoding without development dependencies | Pending |

Run `./check.sh` as the repository's ordinary validation. Heavy model/native probes are explicit development/release checks, not automatic downloads in the unit suite.

## Phase 0b follow-up — 9 October

The [listening record](../../docs/feasibility/listening-2026-10-09/README.md) supersedes the initial lack of user listening/reference comparisons. BF16 with `QWEN_NO_KLEIDI=1` is now the preferred qualification candidate. It is still an experimental configuration; package/download size does not change.

```sh
QWEN_NO_KLEIDI=1 python3 dev/feasibility/benchmark.py "$ENGINE/qwen_tts" "$MODELS" "$EVIDENCE/bf16" --case bf16-en --case bf16-de --case bf16-long-en
```

### Official development reference

Create a Python 3.10 environment in scratch storage, install the versions in `docs/feasibility/listening-2026-10-09/reference-requirements.txt`, and let `REFERENCE_PYTHON` point to that environment's Python. Installation needs network access; inference below uses the already downloaded local packs with Hub/network fetching disabled. Reference CPU float32/eager attention is deliberately conservative and not a native throughput comparator.

```sh
HF_HUB_OFFLINE=1 TRANSFORMERS_OFFLINE=1 "$REFERENCE_PYTHON" dev/feasibility/official_reference.py "$MODELS/cv" "$EVIDENCE/official-ryan.wav"
```

The script records exact versions, settings and WAV hash. `--speaker Aiden` is an optional preset comparison. For official ICL cloning, supply the Base pack plus `--ref-audio LOCAL_WAV --ref-text EXACT_TRANSCRIPT --language German --text TARGET_TEXT`. Both reference arguments are mandatory together; output must not exist. Inspect and listen to the results. Reference generation has a 600-code-token bound; a duration at/above 48 seconds is rejected rather than silently accepted.

### Public human reference

Use the pinned public-domain LJ001-0001 reference and exact transcript in `docs/feasibility/listening-2026-10-09/public-reference.json`. Preserve the original 22.05 kHz WAV; explicitly resample a separate conditioning WAV to 24 kHz mono PCM16. Set `PUBLIC_REFERENCE`, `PUBLIC_TRANSCRIPT` and `PUBLIC_PROFILE` to the appropriate local paths/text; no model training is required.

```sh
ffmpeg -v error -i "$PUBLIC_REFERENCE" -ar 24000 -ac 1 -c:a pcm_s16le "$EVIDENCE/reference-24k.wav"
QWEN_NO_KLEIDI=1 "$ENGINE/qwen_tts" -d "$MODELS/base" --ref-audio "$EVIDENCE/reference-24k.wav" --ref-text "$PUBLIC_TRANSCRIPT" --save-voice "$PUBLIC_PROFILE" --voice-name PublicReference -l English
QWEN_NO_KLEIDI=1 python3 dev/feasibility/benchmark.py "$ENGINE/qwen_tts" "$MODELS" "$EVIDENCE/public-clone" --case bf16-clone-en --case bf16-clone-de --presenter "$PUBLIC_PROFILE"
```

The fresh generation processes receive only the profile, not the recording/transcript. User judged likeness close enough but flagged a bad American accent in German. The later Thorsten German-reference test passed initial listening: recognizable German likeness and usable English with a slight accepted accent. Broader cross-language pronunciation is still a gate; do not promise that changing references automatically removes accents. A consenting user reference remains a later personal-likeness check.

### Signed sandbox diagnostic

`build_sandbox_probe.py` creates a separate local app bundle and harmless outside fixture in a **new** scratch directory. It copies the worker, hardlinks model resources (copy fallback) without modifying them, compiles the Swift parent with a scratch module cache, signs child/parent and verifies deeply/strictly. Models must never be edited in the probe bundle because hardlinks share the source bytes.

```sh
python3 dev/feasibility/build_sandbox_probe.py "$ENGINE/qwen_tts" "$MODELS/cv" "$EVIDENCE/sandbox"
"$EVIDENCE/sandbox/SlopSlideSpeechProbe.app/Contents/MacOS/speech-probe" "$EVIDENCE/sandbox/outside-fixture.txt"
```

Default signing is ad-hoc. To use an available local development identity, add `--identity IDENTITY --identifier UNIQUE_BUNDLE_ID`; keep private identity details out of committed evidence. This creates no installer, upload or Store publication. Both schemes passed on this host. Normal keychain and macOS services access may be needed outside a coding-tool sandbox, which is a distinct boundary from the app's own sandbox.

Expected run: the known existing outside fixture is denied; the bundled worker reads bundled weights, writes a valid WAV in the private container and exits. A sandbox pass requires verifying the fixture is actually readable outside the app; an unsandboxed copy of the parent deliberately exits 2 when that read succeeds. Do not infer enforcement merely from entitlement files. Container output was externally checked as mono PCM16/24 kHz. Only successful lifecycle/CPU synthesis is tested; selected-file access, model/profile installation, GPU, cancellation/crash cleanup and Tauri integration remain pending.

### Additional test cases

| Case | Expected result | Recorded status |
| --- | --- | --- |
| Official missing model / existing output | Argument error before heavy imports; preserve existing bytes | Passed |
| Official only one reference argument / missing reference file | Argument error before model loading | Passed |
| New BF16 stock/clone cases | Valid bounded mono PCM16/24 kHz WAV and same-process census | Passed on recorded host |
| Any clone case without `--presenter` | Argument error before engine launch | Passed |
| Sandbox missing input / existing output directory | Reject before compile/sign; preserve existing output | Passed |
| Development signed bundle and outside-file control | Strict signatures, outside read denied, valid local WAV, helper exits | Passed |
| Unsandboxed parent control | Outside fixture readable; intentional exit 2 | Passed |
| Public-reference clone likeness | Recognizable new text without reference passed again | User: close enough; German accent poor |
| German-reference saved presenter | Fresh-process German likeness and usable English | M recognizable as L; N usable with an accepted slight accent |
| Longer-English pace / official clone accent | Targeted human comparison | J accent persists; K pace approved; German-reference follow-up accepted |

The 1.10× pace sample uses FFmpeg only for an audition. No FFmpeg runtime dependency or app speed control was added. A production pace feature needs native processing, artifact acceptance and duration/cache integration.

## Final resident-model PoC

Phase 0 is now **complete at PoC scope**. Product/device/Store gates above remain later implementation/release checks rather than reasons to extend this spike.

The exact Make compatibility change is retained as `apple-make-compat.patch`. Apply it to a scratch Makefile copy/isolated checkout to reproduce the earlier CPU build; do not silently change upstream numerical defaults.

```sh
python3 dev/feasibility/resident_probe.py "$ENGINE" "$MODELS/cv" "$EVIDENCE/resident"
```

Requires the existing pinned **CPU-only** build objects/static ingot archive, Clang/Accelerate, Python 3.9+ and a new output directory. The script links `resident_probe.c` without modifying upstream source, records source/object/binary hashes, and runs one bounded 120-second process. Its four jobs are English, German, English cancelled after eight generated frames, then the original English again. It checks WAV formats, no cancelled WAV publication and byte-identical English recovery, then unloads/exits. There is no HTTP server, production IPC or Tauri integration. The cancellation callback does not guarantee an error return code; the application must track cancelled jobs explicitly.

| Additional case | Expected result | Result |
| --- | --- | --- |
| Missing engine/model or existing output | Argument error before compile/load; existing files preserved | Passed |
| Apply retained Make patch to pinned original | Exactly reproduces tested compatibility copy | Passed |
| Sequential resident language changes | Valid 24 kHz mono PCM16 WAVs without model reload | Passed |
| Cancel after eight frames, then retry | No cancelled WAV; original English byte-identical | Passed |
| Resident process memory/lifecycle | Record cumulative peak RSS; unload/normal exit; bounded timeout | Passed; 3.037 GiB on this host |

Results: [resident PoC evidence](../../docs/feasibility/resident-2026-10-09/README.md). This answers the narrow model-reuse question; it does not establish a production leak test or full-app fit on low-memory hardware.
