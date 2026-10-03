#include "ghostos/inference.h"
#include <assert.h>
#include <string.h>
static size_t text_length(const char *text) {
    size_t length = 0;
    while (text[length]) ++length;
    return length;
}
static bool contains(const uint8_t *bytes, size_t length, const char *needle) {
    size_t needle_length = text_length(needle), i, j;
    if (needle_length > length) return false;
    for (i = 0; i + needle_length <= length; ++i) {
        for (j = 0; j < needle_length; ++j) if (bytes[i + j] != (uint8_t)needle[j]) break;
        if (j == needle_length) return true;
    }
    return false;
}
static void openai_request_and_grpc_frame(void) {
    const char *body = "{\"model\":\"tiny\",\"prompt\":\"hello\",\"max_tokens\":4,\"stream\":true}";
    const char *zero_tokens = "{\"model\":\"tiny\",\"prompt\":\"x\",\"max_tokens\":0}";
    const uint8_t model[] = {'t', 'i', 'n', 'y'};
    const uint8_t text[] = {'a', '"', 'b'};
    const uint8_t grpc_text[] = {'o', 'k'};
    uint8_t encoded[256];
    ghostos_inference_completion completion;
    bool list_models = true;
    size_t written = 0;
    assert(!ghostos_inference_openai_decode((const uint8_t *)"/v1/completions", 15, (const uint8_t *)body, text_length(body),
        &list_models, &completion));
    assert(!list_models && completion.kind == 0 && completion.stream && completion.max_tokens == 4);
    assert(completion.model_length == 4 && !memcmp(completion.model, "tiny", 4));
    assert(completion.prompt_length == 5 && !memcmp(completion.prompt, "hello", 5));
    assert(ghostos_inference_openai_decode((const uint8_t *)"/v1/unknown", 11, (const uint8_t *)"", 0, &list_models, &completion) == 1);
    assert(ghostos_inference_openai_decode((const uint8_t *)"/v1/completions", 15, (const uint8_t *)zero_tokens, text_length(zero_tokens),
        &list_models, &completion) == 2);
    assert(!ghostos_inference_openai_encode(3, model, 4, text, 3, 1, 9, 2, 1, true, encoded, sizeof encoded, &written));
    assert(contains(encoded, written, "chatcmpl-3") && contains(encoded, written, "a\\\"b"));
    assert(!ghostos_inference_grpc_encode(8, model, 4, grpc_text, 2, 2, 1, true, encoded, sizeof encoded, &written));
    assert(encoded[0] == 0);
    encoded[0] = 1;
    assert(ghostos_inference_grpc_decode(encoded, written, &completion) == 3);
    encoded[0] = 0;
    encoded[4] = (uint8_t)(encoded[4] + 1);
    assert(ghostos_inference_grpc_decode(encoded, written, &completion) == 2);
    assert(ghostos_inference_model_name((const uint8_t *)"", 0) == 2);
    assert(!ghostos_inference_model_name(model, 4));
}
int main(void) {
    openai_request_and_grpc_frame();
    return 0;
}
