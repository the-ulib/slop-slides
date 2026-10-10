/* SlopSlide's private, versioned newline-framed stdio worker. No network server. */
#include "qwen_tts.h"
#include "qwen_tts_kernels.h"
#include "qwen_json.h"
#include "sonic.h"
#include <math.h>
#include <poll.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define RATE 24000
#define LIMIT 562
static volatile sig_atomic_t stopping;
static unsigned long long before, last_report;
static void stop(int sig) { (void)sig; stopping = 1; }
static void error(const char *message) {
    char escaped[2048];
    qwen_json_escape(escaped, sizeof(escaped), message);
    printf("SLOPSPEECH {\"version\":1,\"type\":\"error\",\"message\":\"%s\"}\n", escaped);
    fflush(stdout);
}
static int progress(void *unused) {
    (void)unused;
    struct pollfd p = { .fd = 0, .events = POLLIN };
    if (poll(&p, 1, 0) > 0 && (p.revents & (POLLHUP | POLLERR))) stopping = 1;
    unsigned long long frames = qwen_tts_frames_generated() - before;
    if (frames >= last_report + 12) {
        printf("SLOPSPEECH {\"version\":1,\"type\":\"progress\",\"frames\":%llu}\n", frames);
        fflush(stdout); last_report = frames;
    }
    return stopping;
}
/* Also used by the development sine-wave regression check. */
static int pace(float **audio, int *n, float speed) {
    if (speed == 1.0f) return 1;
    sonicStream stream = sonicCreateStream(RATE, 1);
    if (!stream) return 0;
    sonicSetSpeed(stream, speed);
    int ok = sonicWriteFloatToStream(stream, *audio, *n) && sonicFlushStream(stream);
    int size = sonicSamplesAvailable(stream);
    float *out = size > 0 ? malloc((size_t)size * sizeof(float)) : NULL;
    if (!out || !ok || sonicReadFloatFromStream(stream, out, size) != size) {
        free(out); sonicDestroyStream(stream); return 0;
    }
    sonicDestroyStream(stream); free(*audio); *audio = out; *n = size; return 1;
}
static int self_test(void) {
    const float speeds[] = {0.9f, 1.1f, 1.2f};
    for (int k = 0; k < 3; k++) {
        int n = RATE * 2;
        float *audio = malloc((size_t)n * sizeof(float));
        if (!audio) return 1;
        for (int i = 0; i < n; i++) audio[i] = 0.4f * sinf(2.0f * 3.14159265358979323846f * 440.0f * i / RATE);
        if (!pace(&audio, &n, speeds[k])) { free(audio); return 1; }
        int crossings = 0;
        for (int i = 1; i < n; i++) { if (!isfinite(audio[i])) { free(audio); return 1; } if (audio[i-1] <= 0 && audio[i] > 0) crossings++; }
        double hz = crossings * (double)RATE / n;
        int valid = fabs(n * speeds[k] / (RATE * 2) - 1.0) < 0.04 && fabs(hz - 440.0) < 5.0;
        free(audio); if (!valid) return 1;
    }
    puts("SLOPSPEECH {\"version\":1,\"type\":\"self-test\",\"passed\":true}");
    return 0;
}
#include "profile.h"
int main(int argc, char **argv) {
    if (argc == 2 && !strcmp(argv[1], "--self-test")) return self_test();
    int creating = argc == 6 && !strcmp(argv[1], "--create-profile");
    if (argc != 2 && argc != 3 && !creating) return 2;
    signal(SIGTERM, stop); signal(SIGINT, stop);
    setenv("QWEN_NO_KLEIDI", "1", 1);
    qwen_set_threads(4);
    qwen_tts_ctx_t *ctx = qwen_tts_load_ex(argv[creating ? 2 : 1], 0, 0, 0);
    if (!ctx) { error("Could not load the installed voice pack."); return 1; }
    if (creating) {
        int ok = profile_create(ctx, argv[3], argv[4], argv[5]);
        qwen_tts_unload(ctx);
        if (!ok) { error("Could not create a reusable voice from this recording."); return 1; }
        puts("SLOPSPEECH {\"version\":1,\"type\":\"created\"}"); return 0;
    }
    int cloned = argc == 3;
    if (cloned && !profile_load(ctx, argv[2])) { error("Could not load the saved presenter."); qwen_tts_unload(ctx); return 1; }
    qwen_kleidi_prepack(ctx);
    ctx->temperature = 0.9f; ctx->top_k = 50; ctx->top_p = 1.0f;
    ctx->rep_penalty = 1.05f; ctx->max_tokens = LIMIT;
    ctx->cancel_cb = progress; ctx->cancel_cb_userdata = NULL;
    printf("SLOPSPEECH {\"version\":1,\"type\":\"ready\"}\n"); fflush(stdout);
    char line[32768];
    while (!stopping && fgets(line, sizeof(line), stdin)) {
        if (!strchr(line, '\n')) { error("Oversized request."); break; }
        char *text = NULL, *language = NULL, *speaker = NULL, *output = NULL, *speed_string = NULL;
        const char *why = NULL;
        int valid = qwen_json_extract_string(line, "text", &text, &why) == 1 &&
            qwen_json_extract_string(line, "language", &language, &why) == 1 &&
            qwen_json_extract_string(line, "speaker", &speaker, &why) == 1 &&
            qwen_json_extract_string(line, "output", &output, &why) == 1 &&
            qwen_json_extract_string(line, "pace", &speed_string, &why) == 1;
        char *end = NULL;
        float speed = speed_string ? strtof(speed_string, &end) : 0;
        valid = valid && text[0] && strlen(text) <= 4096 && strlen(output) < 4096 &&
            end && !*end && isfinite(speed) && speed >= 0.9f && speed <= 1.25f &&
            (!strcmp(language, "English") || !strcmp(language, "German"));
        int speaker_id = cloned ? 0 : (speaker ? qwen_tts_resolve_speaker(ctx, speaker) : -1);
        float *audio = NULL; int n = 0;
        if (!valid || speaker_id < 0) error("Invalid speech request.");
        else {
            qwen_tts_set_language(ctx, language); if (!cloned) qwen_tts_set_speaker(ctx, speaker_id);
            ctx->seed = 42; before = qwen_tts_frames_generated(); last_report = 0;
            int rc = qwen_tts_generate(ctx, text, &audio, &n);
            unsigned long long frames = qwen_tts_frames_generated() - before;
            valid = !stopping && rc == 0 && audio && n > 0 && n < 45 * RATE && frames < LIMIT;
            if (valid) for (int i = 0; i < n; i++) if (!isfinite(audio[i])) { valid = 0; break; }
            if (!valid) error("Speech failed or exceeded the segment limit; no recording was accepted.");
            else if (!pace(&audio, &n, speed) || qwen_tts_write_wav(output, audio, n, RATE)) error("Could not save generated audio.");
            else { printf("SLOPSPEECH {\"version\":1,\"type\":\"done\",\"samples\":%d}\n", n); fflush(stdout); }
        }
        free(audio); free(text); free(language); free(speaker); free(output); free(speed_string);
    }
    qwen_tts_unload(ctx); return stopping ? 130 : 0;
}
