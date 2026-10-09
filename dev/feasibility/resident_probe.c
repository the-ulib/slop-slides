/* Diagnostic only: one local CPU context, sequential jobs, no network/IPC server. */
#include "qwen_tts.h"
#include "qwen_tts_kernels.h"
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/resource.h>
#include <time.h>

static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec + t.tv_nsec / 1e9;
}
static long peak(void) {
    struct rusage r;
    getrusage(RUSAGE_SELF, &r);
    return r.ru_maxrss; /* macOS bytes; this probe is Mac-only. */
}
static int cancel_after_eight(void *data) {
    int *checks = data;
    return ++*checks >= 8;
}
int main(int argc, char **argv) {
    if (argc != 3) { fprintf(stderr, "Usage: resident-probe MODEL EMPTY_OUTPUT_DIR\n"); return 2; }
    qwen_set_threads(4);
    double start = now();
    qwen_tts_ctx_t *ctx = qwen_tts_load_ex(argv[1], 0, 0, 0);
    if (!ctx) return 1;
    qwen_kleidi_prepack(ctx);
    ctx->speaker_id = qwen_tts_resolve_speaker(ctx, "ryan");
    if (ctx->speaker_id < 0) { qwen_tts_unload(ctx); return 1; }
    ctx->temperature = 0.9f; ctx->top_k = 50; ctx->top_p = 1.0f;
    ctx->rep_penalty = 1.05f; ctx->max_tokens = 562; ctx->seed = 42;
    printf("PROBE {\"event\":\"loaded\",\"seconds\":%.6f,\"peak_rss_bytes\":%ld}\n", now()-start, peak());
    fflush(stdout);
    const char *en = "Good work needs space. This presentation shows three simple ways to reduce interruptions, protect your attention, and make room for better ideas.";
    const char *de = "Gute Arbeit braucht Ruhe. In dieser Präsentation zeigen wir drei einfache Wege, Unterbrechungen zu reduzieren, die eigene Aufmerksamkeit zu schützen und Raum für bessere Ideen zu schaffen.";
    const char *texts[] = {en, de, en, en};
    for (int i = 0; i < 4; i++) {
        qwen_tts_set_language(ctx, i == 1 ? "German" : "English");
        int checks = 0, n = 0;
        ctx->cancel_cb = i == 2 ? cancel_after_eight : NULL;
        ctx->cancel_cb_userdata = &checks;
        float *audio = NULL;
        unsigned long long before = qwen_tts_frames_generated();
        start = now();
        int rc = qwen_tts_generate(ctx, texts[i], &audio, &n);
        double elapsed = now()-start;
        unsigned long long frames = qwen_tts_frames_generated()-before;
        /* Cancellation is detected by our callback, not by this API's return code. */
        int cancelled = i == 2 && checks >= 8;
        int valid = cancelled ? frames <= 8 : rc == 0 && audio && n > 0 && n < 45 * 24000;
        if (!cancelled && valid) {
            for (int j = 0; j < n; j++) if (!isfinite(audio[j])) { valid = 0; break; }
            char path[4096];
            int len = snprintf(path, sizeof(path), "%s/job-%d.wav", argv[2], i);
            if (len < 0 || len >= (int)sizeof(path)) valid = 0;
            if (valid && qwen_tts_write_wav(path, audio, n, 24000) != 0) valid = 0;
        }
        printf("PROBE {\"event\":\"job\",\"index\":%d,\"seconds\":%.6f,\"samples\":%d,\"generated_frames\":%llu,\"cancelled\":%s,\"valid\":%s,\"peak_rss_bytes\":%ld}\n", i, elapsed, n, frames, cancelled ? "true" : "false", valid ? "true" : "false", peak());
        fflush(stdout);
        free(audio);
        if (!valid) { qwen_tts_unload(ctx); return 1; }
    }
    ctx->cancel_cb = NULL; ctx->cancel_cb_userdata = NULL;
    qwen_tts_unload(ctx);
    return 0;
}
