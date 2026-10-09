# Pinned CPU candidate audit — 9 October 2026

Scope: diagnostic C/Accelerate executable `0e48c8650273f99ff66ca26765978e9c386523d86490686c2ae6118b798816d6`, source revision `ef339be58a778b062e1c14382347964552eae007`. No release package or complete software bill of materials has been produced. This records source evidence and follow-up work rather than certifying redistribution.

| Component in the CPU build | Evidence read | Identified license / release action |
| --- | --- | --- |
| C runtime | root `LICENSE`, build Makefile | MIT; include copyright/license |
| Static ingot library | `third_party/ingot/LICENSE`, Makefile `INGOT_LIB` | MIT; include copyright/license and pin vendored source |
| KleidiAI compiled C/assembly | `third_party/kleidiai/Apache-2.0.txt`, `NOTICE.md`, Makefile `KAI_SRCS`/assembly, per-file SPDX headers | Apache-2.0; preserve licenses/notices; reconcile provenance issue below |
| LZ4 | `vendor/lz4.h` license block, compiled `vendor/lz4.c` | BSD-2-Clause; retain copyright, conditions and disclaimer in binary distribution materials |
| System dependencies | `otool -L` | Only macOS libSystem and Accelerate dynamically linked in this executable |
| Qwen CustomVoice/Base + tokenizer | pinned official pack manifests/model cards, [model source](https://github.com/QwenLM/Qwen3-TTS) | Apache-2.0 identification; final package must include actual applicable licenses/notices for all retained resources |

**Kleidi provenance discrepancy:** the vendored NOTICE describes commit `495f652` and a Q4-only subset, while the actual Makefile also compiles int8 and BF16 kernels. Those BF16 files carry Arm Apache-2.0 SPDX headers (2024–2026), but the NOTICE's stated subset does not account for them. Before release, reconcile the source revision(s) for every compiled vendored kernel and update the consolidated notices. Disabling packing at runtime does not mean compiled dependencies disappear from the binary. This remains a concrete item in release preparation, not a reason to reject the quality experiment.

**22 build warnings classified:** 21 concern unused variables/functions or values assigned but unused. One is `qwen_tts_server.c:3354`: `ctx->layers` is a fixed array (`qwen_tts.h:444`), so its boolean check is always true. The extra check is redundant; this does not validate that all server code is correct. The standalone CLI path does not invoke the HTTP server, but the object is included by the upstream build. Production packaging should build only required worker functionality where practical and run relevant upstream tests after any source/build change. No warnings were silently suppressed and no upstream C source was modified for these probes.

**Build compatibility:** the Apple Make 3.81 hash-escaping workaround lives only in a scratch Makefile copy. The exact compatibility patch is now retained at `dev/feasibility/apple-make-compat.patch` and its application reproduces the tested Makefile copy. Rebuild and requalify checksums/samples for the production package. The diagnostic profile is version 3 and includes model tensors, so its compatibility is tied to the selected engine/model revision.

**Decision:** continue qualification with C CPU BF16 and `QWEN_NO_KLEIDI=1`, retaining default-packed BF16 as a quality comparison. It is now the selected PoC starting point. Do not label it a final release choice until baseline-device, broader quality and dependency provenance gates are cleared. The official Python reference environment and FFmpeg audition tools are development dependencies, not proposed app dependencies. The Rust/Metal alternative still needs equivalent qualification and resolution of its missing standalone license file before adoption.
