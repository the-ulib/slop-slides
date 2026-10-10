/* Private bounded conditioning format, tied to the pinned Base model. No weights. */
#include <stdint.h>
static int profile_load(qwen_tts_ctx_t *ctx, const char *path) {
    if (!ctx->is_base_model) return 0;
    FILE *file = fopen(path, "rb");
    if (!file) return 0;
    char magic[4]; uint32_t dim = 0, text = 0, frames = 0;
    int ok = fread(magic, 1, 4, file) == 4 && !memcmp(magic, "SVP1", 4) &&
        fread(&dim, 4, 1, file) == 1 && fread(&text, 4, 1, file) == 1 && fread(&frames, 4, 1, file) == 1 &&
        dim == (uint32_t)ctx->speaker_enc.enc_dim && dim > 0 && dim <= 2048 && text > 0 && text <= 4096 && frames > 0 && frames <= 800;
    if (ok) {
        ctx->speaker_embedding = malloc((size_t)dim * sizeof(float));
        ctx->ref_text = calloc((size_t)text + 1, 1);
        ctx->cached_ref_codes = malloc((size_t)frames * 16 * sizeof(int));
        ok = ctx->speaker_embedding && ctx->ref_text && ctx->cached_ref_codes &&
            fread(ctx->speaker_embedding, sizeof(float), dim, file) == dim &&
            fread(ctx->ref_text, 1, text, file) == text &&
            fread(ctx->cached_ref_codes, sizeof(int), (size_t)frames * 16, file) == (size_t)frames * 16 && fgetc(file) == EOF;
        if (ok) for (uint32_t i = 0; i < dim; i++) if (!isfinite(ctx->speaker_embedding[i])) ok = 0;
        if (ok) for (uint32_t i = 0; i < frames * 16; i++) if (ctx->cached_ref_codes[i] < 0 || ctx->cached_ref_codes[i] >= 2048) ok = 0;
        if (ok && memchr(ctx->ref_text, 0, text)) ok = 0;
        ctx->cached_ref_n_frames = (int)frames; ctx->voice_clone = 1; ctx->xvector_only = 0;
    }
    fclose(file); return ok;
}
static int profile_create(qwen_tts_ctx_t *ctx, const char *reference, const char *transcript, const char *output) {
    if (!ctx->is_base_model) return 0;
    FILE *textfile = fopen(transcript, "rb");
    if (!textfile) return 0;
    char text[4097]; size_t len = fread(text, 1, sizeof(text), textfile); fclose(textfile);
    if (!len || len > 4096 || memchr(text, 0, len)) return 0;
    uint32_t dim = (uint32_t)ctx->speaker_enc.enc_dim;
    if (!dim || dim > 2048) return 0;
    float *embedding = malloc(dim * sizeof(float)), *audio = NULL;
    int samples = 0, rate = 0, *codes = NULL, frames = 0;
    int ok = embedding && !qwen_extract_speaker_embedding(ctx, reference, embedding) &&
        !qwen_speech_encoder_load(ctx) && !qwen_read_wav(reference, &audio, &samples, &rate) &&
        rate == RATE && samples >= 3 * RATE && samples <= 30 * RATE &&
        !qwen_speech_encoder_encode(ctx, audio, samples, &codes, &frames) && frames > 0 && frames <= 800 && !stopping;
    if (ok) {
        FILE *file = fopen(output, "wb");
        if (!file) ok = 0;
        else {
            uint32_t textlen = (uint32_t)len, nframes = (uint32_t)frames;
            ok = fwrite("SVP1", 1, 4, file) == 4 && fwrite(&dim, 4, 1, file) == 1 &&
                fwrite(&textlen, 4, 1, file) == 1 && fwrite(&nframes, 4, 1, file) == 1 &&
                fwrite(embedding, sizeof(float), dim, file) == dim && fwrite(text, 1, len, file) == len &&
                fwrite(codes, sizeof(int), (size_t)frames * 16, file) == (size_t)frames * 16;
            if (fclose(file)) ok = 0;
        }
    }
    free(embedding); free(audio); free(codes); return ok;
}
