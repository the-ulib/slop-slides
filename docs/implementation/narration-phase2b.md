# Phase 2b — speech provider boundary

Implemented 9 October 2026: **2b.1–2b.3**. Qwen is the normal local provider; a debug-only test-tone provider proves interchange. ElevenLabs (2b.4) and the standalone MCP wrapper (2c) remain planned. See the [architecture proposal](../speech-provider-architecture.md).

## What changed

- `src-tauri/speech-connector` is an independently buildable Rust crate/CLI with no Tauri, deck, slide or manifest dependency. `SpeechProvider` describes voices/languages/readiness/setup/pace and exposes cancellable async synthesis. Qwen owns the pinned packs, resident worker, segmentation and Sonic pace. The app owns deck jobs, accepted-take writes, cache and normalized artifacts.
- The right Narration sidebar selects provider and compatible presenter. Model sizes, voice IDs/hints and pace choices come from backend descriptors. Switching provider preserves existing takes and requires a compatible presenter; it does not silently choose a replacement voice. Playback remains below the slide, including when the provider is unavailable.
- Narration schema v2 adds `speechProviderId`. Version-1 files default to `qwen-local` and migrate in memory without changing their original fingerprint/bytes; the next validated atomic save writes v2. Voice, pace, revisions and take references are preserved. Narration MCP tools and agent instructions handle the new field.
- Legacy Qwen take keys retain their exact previous serialization/hash. Missing provider fields in old take metadata default to Qwen; unchanged accepted audio remains reusable. Other provider keys include provider identity and normalization policy. WAVs remain in the deck's visible `audio/` folder; internal metadata stays in `.slopslide/speech/takes`.
- Adapter artifacts must be regular bounded files inside the job spool, with matching provider/sample metadata. PCM16 decoding/downmixing and ≤24 kHz upsampling happen before immutable publication. Malformed/mismatched/unsupported outputs and cancelled results never replace accepted audio. A changed source/provider rejects late acceptance while retaining the cached take.
- `./check.sh` formats the entire Rust workspace and runs both packages' tests. Model locks explicitly unlock on drop; this prevents a subprocess briefly inheriting an FD from prolonging a finished setup operation's lock. Shared worker leases still exclude pack mutation.

## Verification evidence

`./check.sh`: 851 frontend tests, 308 app Rust tests and 11 connector Rust tests (319 Rust total), four opt-in app tests ignored; typecheck/build, workspace rustfmt and clippy pass. Tests cover alternate voices/pace/setup in the same component, provider/presenter persistence, old takes with unavailable providers, legacy manifest/key compatibility, artifact normalization/path/format checks, cancellation and provider changes during acceptance. The fixture uses 16 kHz output; accepted audio is 24 kHz with the correct duration.

The standalone real-model smoke passed without Tauri or a deck. English 11.76 s, German 10.237458 s, paced English 11.049167 s; all three WAV SHA-256 values exactly match Phase 2. Cancellation after real inference followed by regeneration reproduced the first English hash. Retained [smoke result](narration-phase2b-smoke.json). This establishes preserved output on the tested host, not a new hardware benchmark.

The named macOS debug preview builds successfully. **Post-refactor native playback/reuse acceptance passed on 9 October after the user approved macOS folder access.** The library then opened normally. The existing knowledge-base deck restored Local Qwen, Ryan, English and 1.1× pace with its 32.89625-second recording. Playback advanced to 16 seconds; Pause worked and Skip Back moved it to 1 second. Switching to Chat retained the player. Reuse completed by the next approximately 0.5-second UI observation, with the same accepted take `a986233d-14c7-450d-932e-73fee17e4a28`, WAV SHA-256 `65307647ce33bcd9bce3fb56443a9fcf089e0fd9e8e3392612b6498fbd6b9542` and one metadata take. Reuse migrated narration schema 1 → 2, revision 9 → 10, adding `speechProviderId: qwen-local`; all other existing fields and slide entries were identical. A full app restart restored the recording and playback reached its 32-second end; the player was then reset to the beginning, ready for the user.

The previous attempt stalled on both native library enumeration and a separate shell folder read. The approved-access retry resolves this host's acceptance check; it does not qualify clean-install/device/Store sandbox behavior. Those distribution checks remain open.

## Continue here

1. Read the [connector README](../../src-tauri/speech-connector/README.md). Native playback, seek, restart and legacy reuse are verified above. The debug fixture is opt-in with `SLOPSLIDE_SPEECH_FIXTURE=1`; use a disposable deck and remember it generates a tone, not speech.
2. Next feature: Phase 3's shared timeline and narrated video export, using accepted normalized takes. It should not depend on Qwen model/voice internals.
3. For ElevenLabs, implement another `SpeechProvider`, request/normalize a qualified output format and add backend credentials/cloud consent and mock error tests before a separately authorized paid smoke. For MCP, wrap the same connector task/artifact API; do not relabel the current private worker/CLI as MCP.

The package currently returns completed WAV artifacts, not streaming speech. Rich model options, cloning/profile revisions, MP3/high-rate decoding, cloud account setup/error categories and MCP polling/resources remain their respective later milestones; no generic plugin marketplace or runtime executable downloads were added.
