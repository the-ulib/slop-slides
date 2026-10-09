# Narration feasibility review

8 October 2026; updated 9 October 2026. Companion to the [concept and implementation plan](narrated-video-plan.md).

**Recommendation: proceed with a Mac-first Qwen3-TTS 0.6B prototype, with explicit hardware, quality and Store gates.** The essential technical pieces work in standalone probes. The claim that this will run comfortably on most systems is not yet supported. Phase 0 is complete at the pragmatic PoC scope agreed on 9 October: standalone evidence and initial listening acceptance, not full-product qualification. The plan's [current status and session handoff](narrated-video-plan.md#current-status-and-session-handoff) records completed checks, remaining work and the exact next action. Production phases 1–5 have not started.

## Reproduced behavior

| Requirement | Observed result | Limit of the evidence |
| --- | --- | --- |
| Local stock speech | Native C executable generated English and German WAVs from the official 0.6B CustomVoice pack | Ryan approved in short English/German listening; longer English still slightly slow; small corpus |
| Reusable presenter | Base created a profile from a 13.28-second reference; a fresh process generated different text using only the saved profile | Synthetic and public human references tested; German-reference likeness accepted and English usable with slight accepted accent; English-reference German accent rejected |
| Local processing without API fees | Inference used local model paths and wrote local files; no cloud synthesis service was required | Initial downloads require connectivity; network-denied application operation still needs an integration test |
| Native alternative | Rust/Candle executable compiled with Metal and Accelerate and generated a short WAV on Metal | Only a smoke test; no equivalent memory, quality or cloning qualification |
| HTML slide capture | Hidden WKWebView rendered a static HTML fixture to a verified 1920×1080 PNG | No real deck assets, custom Tauri protocol, remote resources or animation tested |
| Video with narration | AVFoundation produced a 13.30-second, 1920×1080 H.264/AAC MP4 with the generated speech | One slide; multi-slide timing, long jobs, cancellation and disk errors remain untested |

The [working narrated video](feasibility/narrated-slide.mp4) uses real Qwen output; its [original source WAV](feasibility/preset-en.wav) is preserved for the next session's pacing investigation. Its video duration is 13.30 seconds and audio duration 13.28 seconds, a 20 ms difference within one 30 fps frame. `ffprobe` verified codecs/dimensions; FFmpeg decoded the complete file without errors. The decoded frame was visually checked. FFmpeg was used for verification only; the encoder itself uses Apple frameworks.

Additional review samples: [German preset](feasibility/preset-de.wav), [English low-memory configuration](feasibility/lowmem-en.wav), [41-second paragraph](feasibility/long-en.wav), and [saved-profile reuse](feasibility/clone-en.wav). The last sample conditions on a synthetic Ryan reference; it does not demonstrate cloning Uli's voice.

## Memory and speed

Host: Apple M4 Pro, 48 GiB RAM, macOS 26.6.2, arm64. C engine built with Clang and Apple Accelerate, using its default four decoder threads. These are **diagnostic CLI measurements, not production-qualified benchmarks**: one run per case, model loading in each fresh process, warm filesystem cache, dispatch/census instrumentation, a shared workstation, and some concurrent build activity. RTF below 1 means total process time was shorter than the generated audio. There is no resident-worker, cold-disk or baseline-machine benchmark yet.

| Configuration | Language / task | Process wall time | Audio length | Total RTF | Peak process RSS |
| --- | --- | ---: | ---: | ---: | ---: |
| C `--int8`, default packing | English preset | 5.97 s | 13.28 s | 0.449 | 5.500 GiB |
| C BF16 mode, default packing | English preset | 6.81 s | 10.72 s | 0.635 | 4.660 GiB |
| C `--int4`, default packing | English preset | 6.08 s | 12.64 s | 0.481 | 4.745 GiB |
| C `--int8`, default packing | German preset | 5.77 s | 12.80 s | 0.451 | 5.492 GiB |
| C `--int8`, Kleidi packing disabled | English preset | 4.29 s | 12.08 s | 0.355 | 3.514 GiB |
| C `--int8`, Kleidi packing disabled | English paragraph | 26.17 s | 41.12 s | 0.637 | 3.338 GiB |
| C `--int8`, Kleidi packing disabled | Reloaded Base profile | 11.41 s | 11.36 s | 1.005 | 3.591 GiB |

The default configuration retains additional weight representations. `QWEN_NO_KLEIDI=1` disables packing at this pinned revision and substantially reduced measured memory. The census confirms mixed numerical paths: `--int8` does not make the entire pipeline int8. Disabling packing also changes arithmetic and sampled output. Different audio lengths and a single run prevent interpreting this table as a controlled speed ranking.

**Implication:** retain Qwen, but qualify the configuration. The default build exceeds the proposed 4 GiB app-plus-worker budget by itself. The lower-memory configuration is a candidate, with limited headroom once the application and WebKit are included. An M1-class 8 GiB test is required before promising that hardware. A larger-memory launch baseline remains an option if this gate fails.

Structured results, commands, output hashes and same-process operation censuses are stored in [feasibility/measurements.json](feasibility/measurements.json) and [feasibility/census](feasibility/census). Commands use placeholder paths; large models, binaries and private voice profiles are not stored in the repository.

## Speech quality and saved presenters

A local multilingual Whisper base model was used as a rough content check. Default English int8/BF16 and German int8 samples transcribed to the intended words, allowing punctuation differences. Saved-profile output also matched. Some other samples had discrepancies: int4 transcribed “needs” as “meets”; the low-memory short sample omitted the final s in “needs”; the long paragraph included “Do AM” and transcribed “focused” as “focus.” The short Rust Metal sample also transcribed differently from its input. These can be synthesis or recognition errors. The recognizer emitted numerical warnings; none of these results is a quality score or proof of an audible defect.

The initial official comparison is complete (see the 9 October update). Before locking precision, broaden the listening corpus and seeds. Include pronunciation, repeated/missing words, long scripts, audible joins, German and actual speaker likeness. Preserve previews and per-slide regeneration in the UX; a generated file is not automatically an acceptable take.

**Listening feedback: the MP4 demonstration sounds too slow.** Its source contains 22 words in 13.28 seconds, approximately 99 words per minute. A follow-up comparison found the original WAV and MP4 audio both use 24 kHz; decoded AAC matches the source waveform with correlation 0.993 and zero-sample alignment offset in early and late one-second windows. AAC decoding adds about 29 ms of trailing padding, not a progressive timing change. The video export did not introduce the slow pacing. It is present in the generated speech; the cause within synthesis remains unresolved. The later same-text official comparison and approved BF16/pace candidates are recorded below; the exact cause of cross-engine pacing differences has not been established. Natural presentation pacing is an explicit quality gate, and this clip proves the export pipeline only.

The presenter mechanics support the proposed one-time setup. The C runtime saved an approximately 16 MB `.qvoice` profile containing reusable conditioning and some model tensors. A separate process loaded it without the reference WAV/transcript, then spoke a new sentence. This is reference conditioning, with no user-specific model training. Keep profile versions tied to engine/model revisions, retain the private source recording for future recomputation, and avoid promising tiny embedding-only storage. Recording/import, the presenter registry and application default still need implementation. A real consenting speaker must test similarity and English/German reuse before cloning is advertised.

## Model installation and provenance

The official pack manifests report approximately **2.50 GB CustomVoice** and **2.52 GB Base**, including their speech tokenizer. The identical 682 MB tokenizer weights can be shared, yielding approximately **4.33 GB for both packs**. Sizes here are decimal GB; they exclude runtime binaries, temporary installation/update space, generated audio and voice profiles. Runtime int8 quantization does not shrink the downloaded BF16 safetensors in this test.

| Component | Tested revision / SHA-256 |
| --- | --- |
| C runtime | `ef339be58a778b062e1c14382347964552eae007` |
| C executable | `0e48c8650273f99ff66ca26765978e9c386523d86490686c2ae6118b798816d6` |
| Rust/Candle runtime | `711ceee07cad92673f86de8997bdf54c30caa49f` |
| CustomVoice model revision | `85e237c12c027371202489a0ec509ded67b5e4b5` |
| CustomVoice main weights | `bc3c7e785eb961179c25450d1acff03f839e0002f2f3a5aeb67b5735c0fa2adb` |
| Base model revision | `5d83992436eae1d760afd27aff78a71d676296fc` |
| Base main weights | `180b3b10eb1c9f1b4db7806d5475bae3071c0243c299d49926bab1da3b6946f6` |
| Shared speech tokenizer weights | `836b7b357f5ea43e889936a3709af68dfe3751881acefe4ecf0dbd30ba571258` |

Downloaded weight sizes and LFS SHA-256 values were verified. The C built-in self-test passed. Apple's bundled Make 3.81 rejected an unescaped `#` in the upstream Makefile; escaping it in a separate build copy allowed compilation. This fix must be pinned or upstreamed in any production build. The [9 October audit](feasibility/listening-2026-10-09/runtime-audit.md) classifies 22 warnings and confirms tested CPU linkage/licenses. A concrete Kleidi NOTICE versus compiled-kernel provenance discrepancy and the pinned Make compatibility recipe still need release resolution. Reproduction commands are in [dev/feasibility/README.md](../dev/feasibility/README.md).

## Native rendering findings

Two useful issues surfaced in the working snapshot probe:

- Hidden WKWebViews can suspend `requestAnimationFrame`; waiting on it without a deadline hung the first probe. The successful fixture probe waits for fonts/images, then uses a bounded paint wait. Real deck readiness needs more validation.
- Snapshot width is in points: the first result was 3840×2160 on Retina. Explicit normalization produced the required 1920×1080 pixels.

This supports a dedicated render surface. It does not validate the current editor's fixed-delay screenshot approach. Export must still freeze real deck resources, apply final animation state, exclude editor overlays and verify a multi-slide timeline.

## Licensing, platforms and the Mac App Store

The [official Qwen repository](https://github.com/QwenLM/Qwen3-TTS) and [Base model card](https://huggingface.co/Qwen/Qwen3-TTS-12Hz-0.6B-Base) identify Apache-2.0 licensing. The C repository's MIT license was read, along with vendored ingot MIT, KleidiAI Apache-2.0/NOTICE and the LZ4 header's BSD terms. The tested C executable links only system `libSystem` and Accelerate. This is a promising distribution basis, subject to a complete release dependency/notices audit and rights to each voice reference. Rust Cargo metadata declares MIT, but no standalone license file was found at the tested revision; resolve this before adopting it.

The C runtime's pinned [build documentation](https://github.com/gabriele-mastrapasqua/qwen3-tts/blob/ef339be58a778b062e1c14382347964552eae007/docs/building.md) describes Windows through WSL2 beta. Native Windows is a separate inference-portability gate; requiring WSL2 would not satisfy the intended ordinary desktop experience. No Windows, Linux or Intel Mac runtime was executed here.

[Apple's review rules, section 2.4.5](https://developer.apple.com/app-store/review/guidelines/#hardware-compatibility), require Mac App Store sandboxing and a self-contained bundle, and restrict downloaded code/resources that change functionality. [Apple documents sandbox inheritance for child processes](https://developer.apple.com/library/archive/documentation/Miscellaneous/Reference/EntitlementKeyReference/Chapters/EnablingAppSandbox.html), so a native worker is architecturally plausible. The standalone CPU worker now has sandbox evidence below; this does not validate integrated Tauri behavior or model-delivery acceptance. Prefer bundled reviewed model resources for the initial Store experiment; verify any on-demand strategy separately.

**Store gate remains open.** No distribution-signed sandboxed Tauri app/worker was tested. The current Tauri configuration supplies no Store entitlements. The earlier signing query ran under restricted keychain access and was inconclusive; a normal-access query on 9 October found an Apple Development identity. Ad-hoc and Apple Development signed diagnostic bundles passed, but no App Store distribution/provisioning was qualified. Native probes needed normal access to WebKit/encoding services outside the coding tool's restrictive sandbox. That restriction is not evidence of an App Sandbox product failure or success.

The existing application separately relies on externally installed Claude/Codex/Copilot CLIs (`src-tauri/src/agent.rs`) and filesystem access. Store packaging, agent architecture, microphone permission, selected-file access, model loading and child-process cleanup require an integrated review. Qwen does not by itself resolve whole-app Store readiness.

## Qualification update — 9 October 2026

The [listening evidence](feasibility/listening-2026-10-09/README.md) retains clips, exact user feedback, settings, hashes, new measurements and sandbox results. The same pinned C binary/model packs were reused.

**Official quality comparison:** offline `qwen-tts 0.1.1`, PyTorch 2.14.1 and Transformers 4.57.3 generated Ryan with CPU float32/eager attention, four threads, seed 42 and explicit sampling settings matching the native configuration. The official Ryan clip was 12.00 seconds, versus the original native int8 13.28 and default-packed native BF16 10.72. The user preferred native BF16 (B) over both the original int8 (A) and official reference (C). Equal seeds do not give equal random sequences across engines; this is a quality comparison, not numerical parity or proof of the underlying cause of different pacing. No audio speed adjustment was applied to A–I.

**Lower-memory candidate:** BF16 with `QWEN_NO_KLEIDI=1` (D) was judged “very close to B,” with B slightly better. German (E) “sounds great”; the longer English paragraph (F) “also sounds good, but still a bit slow.” Prefer this configuration for further qualification rather than assuming int8 gives the best memory/quality tradeoff. Do not lock a production default yet.

| Lower-memory BF16 task | Process wall time | Audio | Total RTF | Peak process RSS |
| --- | ---: | ---: | ---: | ---: |
| Short English | 5.117 s | 11.76 s | 0.435 | 2.999 GiB |
| German preset | 6.259 s | 11.92 s | 0.525 | 3.008 GiB |
| Longer English | 23.876 s | 42.48 s | 0.562 | 3.043 GiB |
| Reloaded public-reference profile, English | 6.300 s | 11.04 s | 0.571 | 3.082 GiB |
| Reloaded public-reference profile, German | 6.286 s | 13.20 s | 0.476 | 3.099 GiB |

These remain single-run, instrumented CLI measurements on the shared M4 Pro/48 GiB host, with model loading and warm filesystem cache. They omit the application and WebKit. They do not qualify 8 GiB machines, cold startup or a warm resident worker. The stock BF16 census and clone census are retained with [measurements](feasibility/listening-2026-10-09/measurements.json).

**Actual human-reference cloning:** at the user's request, use an openly available reference rather than their voice today. The [LJ Speech maintainer](https://keithito.com/LJ-Speech-Dataset/) identifies the audio/text/metadata as public domain. A pinned Coqui test-data mirror supplied LJ001-0001, a 9.655-second Linda Johnson LibriVox recording, plus its matching transcript. It was resampled from 22.05 to 24 kHz, with real sample-rate conversion. Base saved a profile; separate processes generated new English and German text using only that saved profile. The user found likeness close enough, but the German output has a bad American accent. This establishes useful profile reuse and initial likeness, not acceptable cross-language pronunciation or a clone of the user's voice. Official Base cloning of the same reference/text (J) also had an accent, especially around “r,” according to the user. This makes the defect reproducible beyond the native runtime; it does not prove there is no additional native error. The separate German-reference follow-up below has now passed initial listening; it is not a guaranteed remedy for every presenter. The public recording is diagnostic evidence only, not a proposed shipped presenter. A second test now uses three original German [Thorsten-Voice](https://github.com/thorstenMueller/Thorsten-Voice) phrases under its verified CC0 license. Its 9.54-second conditioning passage created a saved profile; separate processes generated German M (11.44 seconds, 7.438 seconds wall, 3.102 GiB RSS) and English N (9.92 seconds, 5.660 seconds wall, 3.095 GiB RSS). The user judged M recognizably like L and N usable in English with a slight acceptable accent. This passes initial short-sample likeness/usability for this saved presenter. It changes both speaker and reference language, so it cannot isolate the effect of language alone or prove automatic accent removal.

**Signed App Sandbox:** `build_sandbox_probe.py` reproduced a small Swift parent plus bundled C worker/model resources with both ad-hoc and Apple Development signing. Parent entitlements contain only App Sandbox; the helper has App Sandbox plus inheritance. Deep/strict signature checks passed. Both parents were denied reading the known existing outside fixture, then launched the worker, synthesized audio into their private Application Support container and observed normal exit. An unsandboxed negative control could read the fixture and deliberately exited with failure status 2. This demonstrates actual sandbox enforcement alongside successful CPU inference, beyond merely inspecting entitlement files. Tauri integration, model/profile container installation, user-selected import/export, cancellation/crash cleanup, any GPU path and distribution signing/review remain open. Build output, model hardlinks and container paths stay in scratch storage; committed evidence is sanitized.

Broader pace and clone validation remain quality gates; the initial German-reference listening check passed. A development-only 1.10× pitch-preserving `atempo` audition (K) explores whether a modest user pace preference helps the longer paragraph. This is a deliberate transformation of F, not a fix for numerical inference. The user judged K “sounds great,” making 1.10× a supported audition point for a pace control; validate more scripts and the native implementation before release. FFmpeg remains a diagnostic tool; any production pace control needs a validated native/offline implementation and cache/timeline integration.

## Product gates after the PoC

1. **Baseline hardware:** app-plus-worker memory, swap pressure and cancellation on an 8 GiB Apple Silicon Mac; cold start and warm resident-worker throughput; a complete 10-minute deck. Decide the actual supported baseline.
2. **Quality:** broaden the approved longer-English pace adjustment and accepted German-reference clone across scripts/seeds, then later test the user's own recording. Initial official comparison, stock listening and public-reference likeness have evidence; full quality acceptance remains open.
3. **Store architecture:** signed sandbox app with bundled models and worker, private profile storage, CPU/Metal behavior, import/export and microphone paths; audit existing external-agent dependencies.
4. **Real decks:** images/fonts/SVG and final animation state, frozen resources, slide changes, cumulative audio/frame timing, cancellation and failure handling.
5. **Additional platforms:** prove native inference and video packaging on Windows/Linux before making support claims.

The plan now includes measured pack sizes, the memory correction, the provisional C choice, native Windows portability and the hidden-renderer findings. The right-sidebar Chat/Narration UX and saved-presenter concept remain appropriate. Publish this as a scoped proposal with open gates; phase 0 is complete as a PoC, while universal hardware support and Store eligibility remain unverified.

Repository validation: `./check.sh` passed (TypeScript, frontend build, 677 frontend tests, Rust formatting/clippy and 224 Rust tests). Standalone Swift probes compiled and the native encode/decode smoke test passed. No production application behavior was changed by this review.

## Pragmatic PoC closure — 9 October

The user requested that phase 0 remain a proof of concept rather than final-product qualification. The final [resident API probe](feasibility/resident-2026-10-09/README.md) loads one model, generates English/German, cancels a job after eight frames, then reproduces the original English WAV byte-for-byte. It unloaded/exited normally. Peak process RSS was 3.037 GiB; model load 1.612 seconds, first English 6.917 seconds and later repeated English 4.896 seconds for 11.76 seconds audio. Same-process dispatch/census and object hashes are retained. This is a direct API context probe, not production IPC, cold-disk measurement, leak soak or integrated app qualification.

**Verdict: proceed with the PoC configuration.** C CPU BF16 with `QWEN_NO_KLEIDI=1` is the prototype choice. Phase 0's feasibility questions have sufficient evidence. Broad quality/device/Store/release work moves to implementation phases 2–5; no further phase-0 listening or research loop is required. The pinned Apple Make patch is in `dev/feasibility/apple-make-compat.patch`. Large packs, binaries, raw logs and profiles remain outside the repository.
