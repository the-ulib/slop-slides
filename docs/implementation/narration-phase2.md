# Phase 2 — local stock speech

Implemented 9 October 2026 on `codex/narration-poc`. Automated checks and real worker smoke passed. **Native UI acceptance is pending**, so this is not a completed distribution qualification. No PR or push.

## Try it

Run `pnpm app:dev`. The speech build script compiles the pinned C helper on macOS, then starts the app. Developers need Python 3, Git, make and Apple command-line tools; users of the built bundle do not. The generated helper is bundled as a resource; model weights are installed separately. Windows/Linux builds currently support narration editing without native generation.

Open a deck → **Narration** in the right sidebar → **Local speech**. Choose a presenter, language and pace. The default **Presentation · 1.1×** applies pitch-preserving processing to the generated PCM; playback remains at normal speed. Each slide can override the deck language.

On first use, download the **2.50 GB** pinned CustomVoice pack, or choose **Import existing voice pack…**. The verified Phase 0 folder is `/private/tmp/slopslide-qwen-models/cv`, if still present. Import copies and verifies files into the app's data directory, avoiding another download. Cancelled/failed setup retains partial staging for a later retry. Verification occurs before promotion; failed setup is never advertised as installed. Removing the pack leaves deck recordings playable.

Write/draft a script and choose **Generate audio**, for the selected slide or visible scripts in the whole deck. Progress and Cancel appear in Narration. When ready, press **Play below the slide** using the native audio controls. The bar stays visible when switching back to Chat. It shows actual WAV duration. Editing the script, presenter, language or pace labels the accepted audio **Previous recording** until regenerated. Repeating unchanged generation reuses cached audio.

Silent slides, lead/tail pauses and complete narrated playback will be applied by Phase 3's shared timeline. Phase 2 previews the speech recording itself. Personal voice setup remains Phase 4; no microphone access is requested by this phase.

## Storage and runtime

- Pack: app-data `speech/models/custom-voice`; partial setup: `custom-voice.install`. Sizes/checksums are pinned in `src-tauri/speech-worker/custom-voice-pack.json`. Download URLs target an exact Hugging Face revision. Range resumes are checked against the expected Content-Range; servers ignoring Range restart that file.
- Audio: `.slopslide/speech/takes/<uuid>.wav` and matching JSON under the deck. `narration.json` references the accepted take ID. Recordings are immutable 24 kHz mono PCM16; metadata includes source, sample count, checksum, model revision and engine version. Old takes remain retained, including for snapshot references; automatic disk cleanup is deferred.
- Cache identity includes normalized text, language, presenter, pace, model revision and engine version. Visual changes and timeline pauses do not force synthesis. Previously accepted takes from older engine revisions remain playable.
- Generation captures source inputs, saves audio, then merges only the accepted-take reference if the current source still matches. Concurrent unrelated edits survive. Changed-source output stays cached but is not accepted. Errors/cancellation preserve prior accepted audio; temporary segment files are cleaned up.
- One active speech job per app process; OS model locks prevent pack mutation while another process uses it. The resident helper unloads after two idle minutes and is killed on cancellation/error. Its stdin closure also stops generation. No public HTTP server or executable model downloads.
- Long scripts split at sentence/word boundaries into at most 350-character passages, joined with 120 ms silence. Truncated or excessive worker output is rejected; the per-slide recording limit is ten minutes. Broader long-script quality is not yet verified.

## Pinned dependencies

The [Qwen C runtime](https://github.com/gabriele-mastrapasqua/qwen3-tts) is pinned to `ef339be58a778b062e1c14382347964552eae007`, compiled for portable macOS CPU instructions with Accelerate, BF16 and no Kleidi packing. Four threads, seed 42, temperature 0.9, top-k 50, top-p 1.0 and repetition penalty 1.05 retain the accepted PoC configuration.

The [official CustomVoice pack](https://huggingface.co/Qwen/Qwen3-TTS-12Hz-0.6B-CustomVoice/tree/85e237c12c027371202489a0ec509ded67b5e4b5) is pinned to `85e237c12c027371202489a0ec509ded67b5e4b5`. Nine presets are offered, with English/German selection. Ryan and Aiden are English stock voices; preview cross-language pronunciation.

The pace implementation vendors unmodified [Sonic](https://github.com/waywardgeek/sonic/tree/b93885dcb70aae50c6f76b0fe4e0868f029a077e) at `b93885dcb70aae50c6f76b0fe4e0868f029a077e` with its Apache 2.0 license. The build carries Qwen C, Ingot, LZ4, Sonic and model notices into the runtime resource directory. Generated binaries and weights are ignored by Git. Signing, Store packaging and a complete final dependency-notice audit remain Phase 5.

## Verified and remaining

`./check.sh` passed **847 frontend tests and 308 Rust tests**, with four opt-in live tests ignored; typecheck/build, rustfmt and clippy passed. Coverage includes setup cancellation/resume using tiny local files, checksums, interruptible network waits, worker protocol/cancellation, cache corruption/identity, source conflict acceptance, stale UI events/deck loads, saving presenter/pace and preserving prior recordings. The named debug app bundle builds successfully at approximately 53.4 MiB.

`dev/speech-smoke.py` ran against the actual compiled helper and pinned local model. English: 11.76 s; German at 1.1×: 10.237 s; English at 1.1×: 11.049 s. Native DSP sine tests check duration and pitch. Hard cancellation returned in about 0.038 s without an incomplete WAV. A restarted helper reproduced the original English bytes, which also match the Phase 0 recording. Retained [machine-readable results](narration-phase2-smoke.json) describe this one run, not a benchmark. The user approved the earlier 1.1× audition; this Sonic implementation has not yet received a separate human listening judgment.

**Immediate handoff:** the native preview cannot enumerate `/Users/uli/Documents/SlopSlide` and reports `Interrupted system call (os error 4)`. Independent shell directory listings also stall. Restarting the preview did not resolve it; the underlying cause is unknown. Home now shows a contextual message and **Retry opening library**. Resolve the filesystem issue, then verify import → generate → native play/seek → reopen → cached reuse. Do not mark these as passed based on the worker smoke or frontend tests.

Also pending: a full HTTPS download/resume on a clean machine, broader preset/long-script quality, total app memory and baseline hardware. The tested engine used roughly 3 GB in Phase 0; an 8 GB system and Intel Mac were not tested. Keep those checks before support/distribution claims. Once native acceptance passes, proceed to Phase 3's timeline and MP4 integration.
