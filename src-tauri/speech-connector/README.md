# Speech connector

A standalone Rust crate and CLI, independent of Tauri and presentation decks. The app and other consumers use the same `SpeechProvider` interface: capability/voice discovery, async synthesis, progress/cancellation, optional setup/removal and temporary audio artifacts.

The Qwen implementation owns pinned model verification/download/import, warm native worker lifecycle, sentence segmentation and Sonic pacing. Its caller supplies a private model-storage root and the helper executable. The caller also owns each job's spool directory and artifact cleanup. Do not reuse a user-visible deck folder as the connector's model storage.

## Standalone use

From the repository root, build the native helper with `pnpm speech:build`. This development build uses Python; end users use the bundled native helper. The tested helper currently targets macOS CPU. This Rust extraction does not establish native Windows/Linux inference support.

With an already installed pack at `ROOT/models/custom-voice`:

```sh
cargo run --manifest-path src-tauri/Cargo.toml -p speech-connector --bin speech-connector -- describe ROOT HELPER
cargo run --manifest-path src-tauri/Cargo.toml -p speech-connector --bin speech-connector -- synthesize ROOT HELPER REQUEST.json NEW_SPOOL
```

`ROOT` is the private speech-storage directory; `HELPER` is the absolute path to `src-tauri/speech-runtime/slopslide-speech`. `NEW_SPOOL` must not exist; successful output is JSON containing artifact paths, rates and sample counts. Progress goes to stderr. A JSON array of requests reuses the same resident connector. The CLI uses `narration::render`, which also supports explicit pause markers. Library consumers should use that entry point for narration; raw `SpeechProvider::synthesize` accepts plain spoken text and rejects control markers. The library also exposes cancellable setup/import/removal; the small CLI does not yet expose those setup commands.

Example request:

```json
{"text":"Welcome to this presentation.","language":"en","voiceId":"preset:ryan","pace":1.1}
```

No-model interchange proof:

```sh
cargo run --manifest-path src-tauri/Cargo.toml -p speech-connector --bin speech-connector -- fixture REQUEST.json NEW_SPOOL
```

For the fixture use `voiceId: "tone:440"` and `pace: 1.0`. It generates a 16 kHz test tone, **not spoken narration**. SlopSlide exposes it only in debug builds started with `SLOPSLIDE_SPEECH_FIXTURE=1`; the ordinary preview does not list it.

Run the actual-model regression explicitly:

```sh
cargo run --manifest-path src-tauri/Cargo.toml -p speech-connector --example smoke -- ROOT HELPER NEW_SPOOL
```

This checks English/German/1.1× output hashes against retained Phase 2 evidence, warm reuse and cancellation after actual inference followed by successful restart. It does not download models or modify a deck. The repository's `./check.sh` includes both workspace packages and their normal tests; it does not run this heavy smoke.

## Narration controls

Example annotated request:

```json
{"text":"First thought. [pause:800ms] Second thought.","language":"en","voiceId":"preset:ryan","pace":1.1}
```

`Descriptor.narrationControls` and `narrationGuidance` describe the implemented controls and model-specific limits. The reusable `narration::render(provider, request, spool, cancellation, progress)` compiles scripts into spoken passages and exact PCM silence. Supported marker syntax is `[pause:Nms]`, integer N from 1 to 60000, at most 100 markers and ten minutes of total inserted silence per request. The final recording including speech must remain within ten minutes. A pause is additional silence after pace processing; generated speech may include natural pauses. Adjacent markers add their durations; leading/trailing pauses are supported. Empty scripts belong to the host's silent-slide timeline, not speech synthesis.

Unknown alphabetic bracket annotations, incomplete markers and SSML/XML-style tags fail before model invocation. Numeric bracket citations remain literal text. There is no tone/emotion, pronunciation markup, or inline speed/voice control. Qwen 0.6B does not support instruction-driven tone; see its [bundled guidance](guidance/qwen-0.6b.md). The renderer also works with the no-model fixture, and validates/normalizes each passage artifact before concatenating it. Marker-free requests use the unchanged synthesis path.

The SlopSlide narration skill is [bundled in the host](../skills/slopslide-narration/SKILL.md), supplied by `read_narration` together with selected-provider guidance. Other applications can use the renderer/capability data without that skill or any Tauri dependency.

## Saved personal presenters

The optional `SpeechProvider` lifecycle exposes `setup_cloning`, `profiles`, `create_profile`, `preview_profile`, `save_profile`, `discard_profile`, `rename_profile`, `delete_profile` and `unload`. Providers without cloning return unsupported. A draft becomes a selectable voice only after a primary-language preview and save; consumers should require listening before saving. `save_profile(token, replace, name)` accepts an optional final display name; validation and publication happen atomically without rebuilding conditioning. `None` preserves the draft name (or the existing name on replacement). Synthesis binds both opaque `voiceId` and `voiceRevision` from discovery. Use `Descriptor::for_voice` for the selected voice's readiness, engine and model revision; stock and personal packs are independent.

Qwen uses the separately pinned 0.6B Base pack (`data/base-pack.json`) for personal voices. Creation accepts an authorized 3–30 second PCM16 WAV and exact transcript. A compact SVP1 file retains speaker embedding, reference transcript and encoded reference speech; it does not fine-tune or copy model weights. Subsequent synthesis reloads this conditioning without extracting it again. Only one stock/profile worker is resident at a time.

Store references, transcripts, conditioning and registry under the caller's **private** `ROOT/voices`; these never belong in a deck or agent resource. Generated narration belongs to the host's deck audio/history. Rename preserves ID/revision; replacement preserves ID with a new revision; deleting a presenter leaves already generated deck audio intact. Cancellation/failure discards an incomplete creation; hosts must discard abandoned drafts and release staged input recordings. Forced app termination can leave private drafts/temporary recordings; automatic crash cleanup is a later hardening task.

Reproduce the public-reference creation, English/German preview, saved-profile reopen and cancellation/restart checks:

```sh
cargo run --manifest-path src-tauri/Cargo.toml -p speech-connector --example profiles -- ROOT HELPER NEW_OUTPUT
```

This needs the pinned Base pack installed at `ROOT/models/base`; it downloads nothing and modifies no deck. The small CLI exposes discovery/synthesis, while profile management currently uses the reusable Rust interface. Native microphone capture is a host concern, not a dependency of this crate.

## Current boundary and limits

- Contract version 1 uses Rust tasks/cancellation handles; SlopSlide owns job IDs. The planned separate MCP wrapper will map these tasks to pollable jobs/resources. This CLI/private helper protocol is not MCP.
- Voice IDs are opaque and bound to a provider. The Qwen adapter retains legacy `preset:*` IDs for compatibility. Changing provider does not convert a voice profile.
- Qwen returns 24 kHz mono PCM16 WAV. The fixture returns 16 kHz WAV. Host import validates provenance/paths/duration, downmixes PCM16 and upsamples ≤24 kHz to the existing take format. MP3 decoding and a qualified downsampler are deferred to the cloud adapter; unsupported output fails without accepting a take.
- Pace is processed once in the adapter; Qwen uses the existing Sonic implementation. Provider engine revision identifies this processing path. No extra host speed processing occurs.
- Only Qwen and the development fixture are implemented. ElevenLabs authentication/billing/error mapping and MCP packaging remain separate planned work.
- Pack resource metadata is pinned in `data/custom-voice-pack.json` and `data/base-pack.json`. The native engine/Sonic dependencies, notices and build pins remain in `../speech-worker` and `../../scripts/build-speech.py`. This crate is not published; distribution/licensing review still belongs to Phase 5.
