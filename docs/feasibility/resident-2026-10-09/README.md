# Resident model PoC — 9 October 2026

One C process loads the pinned CustomVoice pack once, performs sequential jobs with BF16 and `QWEN_NO_KLEIDI=1`, then unloads and exits. This uses the engine API directly; it does not implement production IPC or a Tauri worker. Tested on the existing M4 Pro/48 GiB host, with warm filesystem cache and operation instrumentation.

| Step | Wall time | Audio | Cumulative peak process RSS |
| --- | ---: | ---: | ---: |
| Model load | 1.612 s | — | 1.283 GiB |
| First English job | 6.917 s | 11.76 s | 3.019 GiB |
| German job, same context | 5.152 s | 11.28 s | 3.035 GiB |
| Cancel next job after eight generated frames | 0.507 s | Discarded; no WAV written | 3.035 GiB |
| English again after cancellation | 4.896 s | 11.76 s | 3.037 GiB |

The first and final English WAVs are byte-identical and also match the user-approved lower-memory BF16 clip D. All completed WAVs are valid mono PCM16/24 kHz. Cancellation produced no published audio; the next job succeeded. The engine cancellation callback can still return a success code, so the application must track cancellation itself rather than treat return code zero as an accepted take. These durations include time before the callback is reached, not user-interaction cancellation latency.

[Measurements and object/binary hashes](measurements.json) and [same-process operation census](census.json) are retained. The upstream source tree was clean; the standalone probe is separate. There was one run, not a soak or baseline-device benchmark. The later repeat benefits from upstream prefix caching; it is not a controlled speedup comparison. Peak RSS is cumulative for this process only. No full-app memory, 8 GiB hardware or cold-disk claim follows.

Reproduction and tests are in [dev/feasibility/README.md](../../../dev/feasibility/README.md). Raw logs/executable/WAVs stay in scratch storage; existing user-approved samples remain in the listening evidence. No additional listening round is required to answer the narrow model-reuse question.
