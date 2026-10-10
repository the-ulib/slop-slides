# Phase 4: reusable personal presenters

Implemented 10 October 2026, at pragmatic PoC scope. The right-sidebar Narration tab now offers **Add my voice…** next to the Presenter picker. A two-step modal handles one-time setup; **Voice & timing → Manage presenters** holds the less frequent management actions.

## User flow

1. Name the presenter and choose the recording's primary language. Download or import the additional **2.52 GB** local Base pack if needed; this is separate from the stock-voice pack.
2. Import a clean PCM16 WAV or record a short passage in the app. Recommended length is 10–20 seconds; accepted range is 3–30 seconds. Listen to the reference, enter/check the exact spoken words, and confirm permission to use the voice.
3. Create a profile, listen to the generated primary-language preview, and save. An optional English/German preview checks cross-language pronunciation. Save is disabled until the primary preview has played. Cross-language accents can remain.
4. The saved presenter appears in the usual picker. Saving selects it for the current slide. Optionally make it the default for **new** decks; existing decks retain their choices. No recording or profile extraction is required for later decks or app restarts.

Management supports rename, replacing a reference, setting/clearing the new-deck default, and confirmed deletion. Rename preserves identity and revision. Replacement keeps identity but creates a new revision for future generation. Existing accepted recordings remain playable, selectable in history, and exportable after replacement/deletion; a missing presenter blocks new generation with recovery guidance.

## Connector/storage boundary

The reusable Rust `SpeechProvider` interface owns optional profile lifecycle operations; the frontend consumes generic capabilities and opaque presenter IDs. Tauri handles jobs, defaults, microphone capture and restricted preview resources. Neither a separate MCP wrapper nor ElevenLabs is required for local profiles; both remain planned adapters around this boundary.

The pinned Base model is `Qwen/Qwen3-TTS-12Hz-0.6B-Base`, revision `5d83992436eae1d760afd27aff78a71d676296fc`. Files are checked against `speech-connector/data/base-pack.json`. The stock engine and cache identity remain unchanged.

The native helper creates a compact SVP1 conditioning file containing the speaker embedding, reference transcript and encoded reference speech. This is reusable conditioning, not fine-tuning. The tested 9.54-second public reference produced **11,871 bytes** of conditioning. One resident worker switches between stock/profile models; there is no simultaneous residency of both packs.

Private data lives in `<app data>/speech/voices/registry.json` and `voices/data/<revision>/` (reference WAV, transcript, conditioning and previews), with private directory permissions and atomic metadata writes. Temporary imported/microphone inputs live in `speech/recordings/`. The application exposes only bounded generated previews and staged-reference playback through special resources; references/conditioning/transcripts never enter deck manifests or agent tools. **Generated narration WAVs and history stay in the deck's `.slopslide/audio` folder.** The manifest stores presenter IDs/names and the selected take; each recording freezes its profile revision.

## Verification and limits

- `./check.sh`: **907 frontend tests, 350 Rust tests passed** (five existing Rust tests ignored), typecheck/build/format/clippy passed.
- Actual-model standalone smoke created a profile from the existing CC0 German reference, generated German/English previews, saved/reopened it in a fresh connector, preserved revision on rename, and recovered after cancellation with identical output. The user judged both new previews usable. [Public listening evidence and hashes](phase4-listening/results.json); [German preview](phase4-listening/preview-de.wav), [English preview](phase4-listening/preview-en.wav).
- Stock regression: all three retained English/German/pace WAV hashes match earlier evidence; cancellation after inference and restart passed.
- Native packaged-app acceptance passed: Base pack import, public-reference import, exact transcript, profile creation, audio playback, listening-gated save, another deck with the new default, rename, clearing the temporary default, full app restart, picker rediscovery and fresh synthesis without importing a reference again. Narrated-deck playback and MP4 export passed: **4.766667 seconds, 143 frames, 1920×1080, H.264/AAC, 24 kHz mono**. [Native evidence](phase4-listening/native-results.json).
- A native status-lock deadlock found during acceptance was fixed by releasing the provider-registry guard before collecting status; a regression test checks snapshot lock release. A frozen cloned take exports without a live profile/model in the video test.

Microphone permission-denied/setup-failure/stop/cancel/30-second cleanup paths are covered with tests. The macOS bundle includes `NSMicrophoneUsageDescription` and the audio-input entitlement. **Live microphone recording and permission acceptance still need a human check**; the user requested using a public reference while their voice is affected by a cold. This is not an App Store qualification claim. Sources: [Tauri macOS bundling](https://v2.tauri.app/distribute/macos-application-bundle/), [Apple media-capture authorization](https://developer.apple.com/documentation/bundleresources/requesting-authorization-for-media-capture-on-macos).

The preview app's library listing waited in macOS directory access during these checks; creating/opening the disposable test decks worked. Restart reuse was therefore checked in a new test deck rather than through Recent decks. Directory-permission/clean-install qualification remains open. The existing narration acceptance deck retained its original accepted recording. The public test presenter remains available in the preview app; the temporary new-deck default was cleared. Replacement/deletion and frozen-recording export after profile removal are covered by unit tests, not a destructive native test.

Imported formats are deliberately limited to PCM16 WAV. Transcript entry is manual; there is no transcription download or service. Creation errors/cancellation clean up incomplete profiles, closing the wizard discards its draft and releases its staged input. A forced app termination can leave private drafts/temporary inputs; crash cleanup, AudioWorklet capture, broader voice-quality/deck/device testing and full distribution/privacy review remain hardening tasks. Only public test audio is retained in this repository; no saved personal profiles or model weights are committed.

## Resume

Run `pnpm app:dev`, open a slide's Narration tab and use **Add my voice…**. For a no-microphone reproduction use `docs/feasibility/listening-2026-10-09/l-german-public-reference.wav` with this exact German transcript:

> Eure Schoko-Bonbons sind sagenhaft lecker! Europa und Asien zusammengenommen wird auch als Eurasien bezeichnet. Euer Plan hat ja toll geklappt.

Build the helper with `pnpm speech:build`; reproduce the opt-in actual-model lifecycle using `speech-connector/examples/profiles.rs` as documented in the connector README. Next feature is **Phase 4b's bounded tone feasibility gate**. Do not add tone controls to the current 0.6B adapter without a listening/resource/runtime pass. ElevenLabs/MCP and Phase 5 remain separate planned work.
