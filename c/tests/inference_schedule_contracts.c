#include "ghostos/inference_schedule.h"
#include <assert.h>
#include <string.h>
static void scheduling_keeps_duplicate_models_ahead_of_invalid_limits(void) {
    const uint8_t tiny[] = {'t', 'i', 'n', 'y'};
    const uint8_t other[] = {'o', 't', 'h', 'e', 'r'};
    ghostos_inference_model models[1];
    ghostos_inference_execution executions[1];
    size_t slot = 0, model_slot = 0, execution_slot = 9, total = 0;
    uint64_t handle = 0, committed = 0;
    memset(models, 0, sizeof models);
    memset(executions, 0, sizeof executions);
    assert(!ghostos_inference_register_model(models, 1, tiny, 4, 8, 1, 2, &slot));
    assert(slot == 0);
    assert(ghostos_inference_register_model(models, 1, tiny, 4, 0, 1, 1, &slot) == 2);
    assert(ghostos_inference_register_model(models, 1, other, 5, 0, 1, 2, &slot) == 1);
    assert(ghostos_inference_prepare_start(models, 1, executions, 1, 7, tiny, 4, 0, 4, 9, &model_slot, &execution_slot, &total) == 1);
    assert(!ghostos_inference_prepare_start(models, 1, executions, 1, 7, tiny, 4, 4, 2, 9, &model_slot, &execution_slot, &total));
    assert(model_slot == 0 && execution_slot == 0 && total == 6);
    assert(!ghostos_inference_commit_start(executions, 1, execution_slot, model_slot, 7, 4, 2, &handle));
    assert(handle == ((uint64_t)1 << 32));
    assert(ghostos_inference_prepare_start(models, 1, executions, 1, 7, tiny, 4, 4, 2, 9, &model_slot, &execution_slot, &total) == 1);
    assert(ghostos_inference_unregister_model(models, 1, executions, 1, tiny, 4) == 3);
    assert(ghostos_inference_prepare_tokens(executions, 1, handle, 3, 4, 10, &committed) == 1);
    assert(!ghostos_inference_prepare_tokens(executions, 1, handle, 1, 4, 10, &committed));
    assert(committed == 5);
    assert(ghostos_inference_prepare_tokens(executions, 1, handle, 1, 6, 10, &committed) == 6);
    assert(ghostos_inference_finish(executions, 1, handle - 1) == 5);
    assert(!ghostos_inference_finish(executions, 1, handle));
    assert(!ghostos_inference_unregister_model(models, 1, executions, 1, tiny, 4));
    assert(ghostos_inference_unregister_model(models, 1, executions, 1, tiny, 4) == 4);
}
int main(void) {
    scheduling_keeps_duplicate_models_ahead_of_invalid_limits();
    return 0;
}
