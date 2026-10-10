---
name: slopslide-narration
description: Write and edit spoken presentation narration in SlopSlide with validated timing markers and the selected speech connector's capabilities.
---

Use read_narration before editing. Its response includes the file fingerprint, this guidance, and provider-specific narration controls/guidance. Resolve each slide's provider/language/voice/pace overrides before choosing controls; do not infer capabilities from the provider's brand name. Unknown or absent capabilities mean plain narration only.

Write natural spoken explanations for the requested audience and slides. Spell out ambiguous numbers, abbreviations and pronunciation in the spoken script when useful; do not alter the slide's visible text. Use punctuation for natural phrasing, without promising an exact duration.

For a deliberate pause, use [pause:800ms] in the script: integer milliseconds from 1 to the connector's maxPauseMs, up to maxMarkers markers per slide. Use pauses sparingly between thoughts. The renderer inserts this exact silence in addition to natural pauses in generated speech. Include spoken words; use an empty script with silentDurationMs for a silent slide. No invented bracket stage directions, SSML, tone, emphasis, pronunciation tags or inline voice/speed changes. Only use controls the connector and renderer explicitly support. Review the returned provider guidance for model-specific limits; Qwen 0.6B tone instructions are unsupported.

Save through write_narration using the returned fingerprint as base. Preserve other scripts, removed-slide entries, settings, timing and acceptedTakeId references. Capability/guidance metadata is not part of the manifest. On a conflict, reread and merge the requested edits; do not bypass the tool with file writes. Mark changed scripts for review with reviewedSlideHash: null. Ask the user to review the script and listen after generating audio; do not claim pauses or voice delivery were verified without playback or audio evidence.
