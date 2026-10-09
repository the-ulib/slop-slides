# Phase 3 — narrated preview and macOS MP4

Implemented on `codex/narration-poc`, 9 October 2026. **Native preview/export/cancellation acceptance and the 10-minute standalone native export passed.** No PR or push.

## Try it

Give each visible slide either an accepted recording matching its current script/voice/language/pace, or an explicit **Silent slide duration** in Narration. Hidden slides are excluded. Changed scripts block video preparation until their audio is regenerated; visual changes produce review notes.

Choose **Preview narrated deck** in the right Narration sidebar. Preparation creates a frozen render/audio job. Play, pause or seek using the playback controls; the picture follows the exported timeline. Choose **Export MP4**, or use **Export → Narrated MP4**, then choose a destination. Progress and Cancel are available throughout preparation/encoding. Closing a ready preview removes its scratch job; reopening prepares the latest revision.

![Actual native narrated-deck preview, with play/pause and seek controls](narration-phase3-native.png)

The native video helper requires macOS 12 or newer. MP4 uses static 1920×1080 slides, 30 fps, H.264 video and AAC audio. Animations are flattened at their final state. Embedded video/audio/canvas/iframe/SVG animation receive a flattening note; embedded sound is excluded. External scripts and responsive `srcset` images report unsupported content instead of silently claiming animated support.

## Implementation

- `src-tauri/src/video.rs` builds one provider-independent integer timeline from validated accepted WAV sample counts at 24 kHz. Lead-in/tail pauses are exact samples. Silent duration is the entire silent slide. Frame boundaries round cumulative time; the final audio/video duration pads upward to a complete frame, preventing clipped endings.
- The app copies HTML/assets, checks their fingerprints and narration version again, and rejects mixed revisions during freezing. Accepted WAV checksums bind audio to that snapshot. Later source edits cannot change prepared pictures or sound. Job copies live in the application's cache, not the deck; accepted recordings remain in its visible `audio/` folder.
- `src-tauri/video-worker/main.swift` is a bundled WebKit/AVFoundation helper. It renders a separate never-shown 1920×1080 WKWebView, normalizes Retina snapshots to exact pixel dimensions, waits for fonts/images/backgrounds and layout, and uses the previously validated bounded hidden-view paint fallback. Editor/review overlays are absent. Local assets are copied; ordinary remote HTML/CSS/font/image references are frozen to data URLs before rendering and CSP then denies network. Export preparation needs network if the deck references external resources; fully local decks work offline. Source deck files are not rewritten.
- Encoding holds one reusable pixel buffer at a time and waits for writer readiness, then uses AVFoundation to finalize H.264/AAC. Destination-volume scratch output is renamed only after successful validation. Cancellation kills/reaps the helper and removes temporary files; failed output preserves an existing destination. Progress IPC is local JSON stdout, not a server or an MCP dependency.
- The frontend renders preparation/errors/progress, a small transport over native HTML audio and frozen PNGs; its preview clock uses the same 30 fps boundaries. The memoized audio player stays independent of slide redraws, and slide pictures update only at slide boundaries; elapsed time follows media events. It knows no Qwen engine or model facts. Windows/Linux and browser preview report that narrated video requires macOS.

The native API references are Apple’s [WKWebView documentation](https://developer.apple.com/documentation/webkit/wkwebview) and [AVAssetWriter pixel-buffer interface](https://developer.apple.com/documentation/avfoundation/avassetwriterinputpixelbufferadaptor).

## Verification

`./check.sh` passes: 864 frontend tests, 319 app Rust tests plus 11 connector tests (330 Rust total); five opt-in app tests ignored. Typecheck/build, Rust formatting and clippy pass. Added cases cover cumulative rounding over 1,000 slides, lead/tail/silent PCM placement and consecutive spoken takes, hidden/orphan slides, stale/missing speech, snapshots surviving source edits, cancellation, symlink/path rejection, protocol exposure, injected ENOSPC and preservation/cleanup of a previous movie. Frontend tests cover seek-driven pictures, export/save cancellation, late preparation cleanup, readiness errors and menu entry.

The opt-in native test rendered/encoded a 10-minute two-slide fixture containing an existing accepted real Qwen recording, then silence. On the tested host it completed in 76.73 s. Independent development-only FFprobe reported **600.000 s, 18,000 frames, 1920×1080, 30 fps, H.264/AAC, 24 kHz mono**, 3,242,435 bytes. Decoded frames switched from slide A to B exactly at frame 344 (11.466667 s); the immediately preceding frame retained A. AAC leading/silent-slide sections decoded to zero; speech correlation against the original PCM was 0.98968. Native snapshots were nonblank with the expected colors and revealed heading. FFmpeg/FFprobe were used only for verification, never required by the app.

Reproduce: `pnpm video:build`, then `SLOPSLIDE_VIDEO_EVIDENCE=/tmp/slopslide-video-evidence cargo test --manifest-path src-tauri/Cargo.toml video::tests::native_ten_minute_render_and_encode -- --ignored --nocapture`. Native tests need access to macOS WebKit/AVFoundation services; a restricted terminal sandbox blocked the first attempt, while the authorized native retry passed. The helper is built automatically by `pnpm app:dev` / `pnpm app:build`.

Native app acceptance used a disposable six-slide copy of the knowledge-base deck; the original was unchanged. The readiness check blocked a missing silent duration; entering 5 seconds in Narration and reopening prepared all six slides. External Google Fonts were localized and the final asset image rendered correctly. Native playback advanced across the speech/silence boundary and seeking selected the correct slide. Native export produced **58.666667 s / 1,760 frames / 1920×1080 / H.264+AAC**, 8,829,307 bytes. The app was resized during the export; output dimensions stayed fixed. Cancelling a second export over an existing test MP4 preserved its SHA-256 and removed both the render spool and destination scratch directory. A fresh preview was prepared afterward. WebKit’s built-in controls intermittently displayed a stale time despite correct audio and pictures, so the preview uses a small explicit play/pause/seek bar driven by the audio clock. The final rebuilt app’s elapsed time advanced from 0:09 to 0:41, automatically showing slide 3, and a backward seek to 0:29 restored slide 1. Pause/play behaved correctly. The screenshot above captures that final transport. Retained [verification summary](narration-phase3-smoke.json).

## Remaining qualification

Actual disk-full volume behavior, unusual CSS/SVG/generated media, crash/forced-quit recovery, baseline-device memory/performance and signed Store sandbox output permissions remain distribution/extended tests. ENOSPC is covered by fault injection; it is not evidence from a full physical disk. Scratch jobs can remain after a forced process kill and are safe to remove from the app cache. Do not claim general animated export or Store readiness from this PoC.

Next feature after native acceptance: Phase 4's saved personal presenters. ElevenLabs (2b.4) and the standalone speech MCP wrapper (2c) remain planned and use the existing speech connector boundary.
