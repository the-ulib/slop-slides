# Narration workflow UI review

10 October 2026. Proposal only; no production frontend or speech/video implementation changed.

Open [the two alternatives](narration-workflow-review.html) in a browser. The carousel shows one complete app layout at a time. The [editable fragment](narration-workflow-review.fragment.html) is retained beside the standalone version and linked from the narration plan.

| Alternative | Selected slide | Whole presentation |
| --- | --- | --- |
| Slide first | Presenter → script → generate/listen; recordings and timing closed initially | Preparation, preview and export below the slide controls |
| Separate workspace | Script → presenter → generate/listen; recordings and timing closed initially | Separate Presentation view with readiness, generate missing audio, preview and export |

Both retain left thumbnails, center slide/playback and right Chat/Narration tabs. The actual app continues to start on Chat; the mockups open Narration for comparison. **The left thumbnails are the only slide navigation.** User feedback during review identified the duplicate slide list in the original Presentation view as unnecessary. It was removed; audio readiness appears on the existing thumbnails and the Presentation view shows only an aggregate readiness summary and batch/video actions.

Review these tasks before choosing a layout:

1. Create a named voice from an imported reference or recording. Find the instructions before starting, edit the name at preview, record again and save.
2. Edit one slide's script and generate audio. Other slides retain their settings/recordings. Expand Recordings, audition an older take, then select it; a take with different words explicitly restores its original script.
3. Open whole-presentation preview/export. Missing audio must be prepared first; existing selected takes are preserved. Blank scripts use five seconds of silence.

Prototype limitations: no microphone/file access, synthesis, actual audio, presenter persistence, network service or video file. Fixed example durations/readiness illustrate the flow rather than measurements. The recording simulation skips the real warm-up/capture timer. Existing signed-provider/model setup, conflict handling, error/cancellation and detailed take provenance remain implementation requirements; this review changes visual hierarchy, not those contracts. The nine real stock presets are represented by two examples. Provider selection remains secondary under Voice & timing; no ElevenLabs/MCP/tone support is implied.

Provisional recommendation: Separate workspace. Await user preference and one focused refinement before implementation. The cleanup should reuse existing frontend state and backend commands, preserve session tab/draft behavior, per-slide settings, five-second silent defaults, immutable history and explicit accepted-take export.

Run the prototype cases with `node --test docs/mockups/narration-workflow-review.test.mjs`. Eleven cases cover both variants: editing/generation/history restore, recording preparation/retry/name validation, import/save/per-slide selection, export readiness, silent slides/Chat drafts and a single slide navigation. Repository `./check.sh` also passed (924 frontend tests, 351 Rust tests; 5 existing Rust tests ignored).

To regenerate the standalone version after editing the fragment, run the visualization skill's `scripts/render.py` with the fragment as its input, `narration-workflow-review.html` as its destination and `--force`. Only optional widget-state host calls are used; exported controls stay local.
