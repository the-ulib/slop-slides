# Phase 0b listening and sandbox evidence — 9 October 2026

These are diagnostic samples, not production features. All synthesis used the previously pinned 0.6B packs and seed 42. Native samples use the pinned C executable and its default four CPU threads. A–J play at their generated speed; K is deliberately transformed to 1.10×. Play files at 1× in the player.

| Clip | Source / settings | Duration | User judgment |
| --- | --- | ---: | --- |
| [A](../preset-en.wav) | Native Ryan, original int8/default packing | 13.28 s | B preferred in A/B/C comparison |
| [B](b-native-bf16-ryan.wav) | Native Ryan, BF16/default packing | 10.72 s | “I like B best” |
| [C](c-official-ryan.wav) | Official Ryan, CPU float32/eager | 12.00 s | B preferred |
| [D](d-native-bf16-lowmem.wav) | Native Ryan, BF16, `QWEN_NO_KLEIDI=1` | 11.76 s | “D is very close to B. B sounds slightly better.” |
| [E](e-native-bf16-german.wav) | D configuration, German | 11.92 s | “E sounds great.” |
| [F](f-native-bf16-paragraph.wav) | D configuration, longer English | 42.48 s | “F also sounds good, but still a bit slow” |
| [G](g-public-reference.wav) | Original public-domain LJ001-0001, 22.05 kHz | 9.655 s | Reference for comparison |
| [H](h-public-clone-en.wav) | Base BF16/no Kleidi, saved public-reference profile, new English | 11.04 s | G/H/I: “I think it is close enough.” |
| [I](i-public-clone-de.wav) | Same saved profile, new German | 13.20 s | “But the german voice has a bad american accent.” |
| [J](j-official-public-clone-de.wav) | Official Base ICL clone, same reference/text as I | 13.60 s | Accent persists, especially “r” |
| [K](k-paragraph-pace-110.wav) | F processed with pitch-preserving `atempo=1.1` | 38.606 s | “K sounds great!” |

Interpretation: native BF16 is the preferred quality reference; BF16 without Kleidi packing is the preferred lower-memory qualification candidate. The short stock English/German results are encouraging. A 1.10× audition resolves the longer-English pace preference for this sample; the German-reference clone's likeness is accepted, and its English output is usable with an accepted slight accent. The English-reference German clone still has the previously rejected American accent. Production pace processing still needs implementation/validation. One reference/seed and one listener do not qualify an entire product corpus. The reserved official Aiden sample has not been presented for listening; no user approval is recorded for it.

[Measurements](measurements.json) contain single-run peak RSS/wall time, placeholder commands, WAV hashes and exact listening replies. [Official Ryan settings](c-official-ryan.json) and [official German clone settings](j-official-public-clone-de.json) include versions/sampling settings. [Reference packages](reference-requirements.txt) record the Python environment; use it only for development comparisons, never as an end-user dependency. Equal seeds across runtimes are not numerical parity.

[Public-reference provenance](public-reference.json) records the pinned sample/transcript mirror, resampling and profile creation. The [LJ Speech maintainer](https://keithito.com/LJ-Speech-Dataset/) identifies the recordings, text and metadata as public domain. This is an internal diagnostic reference, not a proposal to distribute a presenter named after the speaker. The roughly 16 MB profile and full model packs remain in scratch storage. User's own recording was deferred at their request.

[Sandbox results](sandbox-results.json) record ad-hoc and Apple Development signed standalone CPU-worker passes with a denied outside-file negative control. Integrated Tauri, selected-file export, cancellation/crash cleanup, GPU and App Store distribution acceptance remain untested.

Resume from the [plan status](../../narrated-video-plan.md#current-status-and-session-handoff) and [probe instructions](../../../dev/feasibility/README.md). Do not rerun completed quality comparisons unless settings change. J/K feedback is recorded: pace audition approved, official German clone also accented. L/M/N feedback is also recorded: German likeness accepted and English usable with a slight accent. The resident-context PoC has also passed; phase 0 is complete at PoC scope. Next is phase 1; broader quality, baseline-device and integrated sandbox checks belong to later implementation milestones.

[Runtime dependency/build audit](runtime-audit.md) records the tested linkage, warning classification and a concrete Kleidi vendored-provenance discrepancy to resolve before release.

Target-language reference follow-up: [L](l-german-public-reference.wav) is a 9.54-second concatenation of three original Thorsten-Voice German phrases; [M](m-german-reference-clone-de.wav) is new German (11.44 s) and [N](n-german-reference-clone-en.wav) new English (9.92 s) from its saved Base profile. [Source/license/transcript provenance](german-public-reference.json) is pinned. The speaker explicitly publishes this corpus for open TTS use under CC0. User feedback: “M sounds recognizably like L. N is also usable in English - with a slight accent though, but this time is fine.” Initial short-sample cloning acceptance passed for this reference. Switching reference and speaker together is not a controlled same-speaker language comparison. Broader scripts/seeds and the user's own reference remain untested.
