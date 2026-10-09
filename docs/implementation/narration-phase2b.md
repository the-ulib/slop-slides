# Phase 2b — speech provider boundary

Implemented 9 October 2026: **2b.1–2b.3**. Qwen is the normal local provider; a debug-only test-tone provider proves interchange. ElevenLabs (2b.4) and the standalone MCP wrapper (2c) remain planned. See the [architecture proposal](../speech-provider-architecture.md).

## What changed

- `src-tauri/speech-connector` is an independently buildable Rust crate/CLI with no Tauri, deck, slide or manifest dependency. `SpeechProvider` describes voices/languages/readiness/setup/pace and exposes cancellable async synthesis. Qwen owns the pinned packs, resident worker, segmentation and Sonic pace. The app owns deck jobs, accepted-take writes, cache and normalized artifacts.
- The right Narration sidebar selects provider and compatible presenter. Model sizes, voice IDs/hints and pace choices come from backend descriptors. Switching provider preserves existing takes and requires a compatible presenter; it does not silently choose a replacement voice. Playback remains below the slide, including when the provider is unavailable.
- Narration schema v2 adds `speechProviderId`. Version-1 files default to `qwen-local` and migrate in memory without changing their original fingerprint/bytes; the next validated atomic save writes v2. Voice, pace, revisions and take references are preserved. Narration MCP tools and agent instructions handle the new field.
- Legacy Qwen take keys retain their exact previous serialization/hash. Missing provider fields in old take metadata default to Qwen; unchanged accepted audio remains reusable. Other provider keys include provider identity and normalization policy. WAVs remain in the deck's visible `audio/` folder; internal metadata stays in `.slopslide/speech/takes`.
- Adapter artifacts must be regular bounded files inside the job spool, with matching provider/sample metadata. PCM16 decoding/downmixing and ≤24 kHz upsampling happen before immutable publication. Malformed/mismatched/unsupported outputs and cancelled results never replace accepted audio. A changed source/provider rejects late acceptance while retaining the cached take.
- `./check.sh` formats the entire Rust workspace and runs both packages' tests. Model locks explicitly unlock on drop; this prevents a subprocess briefly inheriting an FD from prolonging a finished setup operation's lock. Shared worker leases still exclude pack mutation.

## Evidence and remaining native check

`./check.sh`: 851 frontend tests, 308 app Rust tests and 11 connector Rust tests (319 Rust total), four opt-in app tests ignored; typecheck/build, workspace rustfmt and clippy pass. Tests cover alternate voices/pace/setup in the same component, provider/presenter persistence, old takes with unavailable providers, legacy manifest/key compatibility, artifact normalization/path/format checks, cancellation and provider changes during acceptance. The fixture uses 16 kHz output; accepted audio is 24 kHz with the correct duration.

The standalone real-model smoke passed without Tauri or a deck. English 11.76 s, German 10.237458 s, paced English 11.049167 s; all three WAV SHA-256 values exactly match Phase 2. Cancellation after real inference followed by regeneration reproduced the first English hash. Retained [smoke result](narration-phase2b-smoke.json). This establishes preserved output on the tested host, not a new hardware benchmark.

The named macOS debug preview builds successfully. **A new native UI playback/reuse check is still pending:** opening/restarting the preview returned the library screen, but enumeration of the existing Documents/SlopSlide folder stalled. A separate shell directory listing stalled too, including outside the execution sandbox. This is consistent with the previously recorded library/filesystem issue and does not identify its cause; do not mark the post-refactor native UI flow as passed. No user deck was modified during this check. Retry when the library read recovers; verify provider/presenter/pace, play the existing recording, then reuse its unchanged take ID/checksum. Before release, keep the existing clean-install/device/sandbox checks.

## Continue here

1. Read the [connector README](../../src-tauri/speech-connector/README.md). Complete the native UI recheck above when the library is readable. The debug fixture is opt-in with `SLOPSLIDE_SPEECH_FIXTURE=1`; use a disposable deck and remember it generates a tone, not speech.
2. Next feature: Phase 3's shared timeline and narrated video export, using accepted normalized takes. It should not depend on Qwen model/voice internals.
3. For ElevenLabs, implement another `SpeechProvider`, request/normalize a qualified output format and add backend credentials/cloud consent and mock error tests before a separately authorized paid smoke. For MCP, wrap the same connector task/artifact API; do not relabel the current private worker/CLI as MCP.

The package currently returns completed WAV artifacts, not streaming speech. Rich model options, cloning/profile revisions, MP3/high-rate decoding, cloud account setup/error categories and MCP polling/resources remain their respective later milestones; no generic plugin marketplace or runtime executable downloads were added.
