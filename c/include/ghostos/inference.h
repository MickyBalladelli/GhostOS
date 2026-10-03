#ifndef GHOSTOS_INFERENCE_H
#define GHOSTOS_INFERENCE_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 unsupported endpoint, 2 invalid request,
   3 unsupported protocol, 4 buffer too small.
   Completion kind: text=0, chat=1. */
#define GHOSTOS_INFERENCE_MODEL 96u
typedef struct {
    const uint8_t *model;
    const uint8_t *prompt;
    size_t model_length, prompt_length;
    uint32_t max_tokens;
    bool stream;
    uint8_t kind;
} ghostos_inference_completion;
int ghostos_inference_model_name(const uint8_t *value, size_t length);
int ghostos_inference_openai_decode(const uint8_t *path, size_t path_length, const uint8_t *body, size_t body_length,
    bool *list_models, ghostos_inference_completion *completion);
int ghostos_inference_openai_encode(uint64_t id, const uint8_t *model, size_t model_length, const uint8_t *text,
    size_t text_length, uint8_t kind, uint64_t created_at, uint32_t prompt_tokens, uint32_t completion_tokens,
    bool finished, uint8_t *destination, size_t capacity, size_t *written);
int ghostos_inference_grpc_encode(uint64_t id, const uint8_t *model, size_t model_length, const uint8_t *text,
    size_t text_length, uint32_t prompt_tokens, uint32_t completion_tokens, bool finished, uint8_t *destination,
    size_t capacity, size_t *written);
int ghostos_inference_grpc_decode(const uint8_t *frame, size_t length, ghostos_inference_completion *completion);
#endif
