#ifndef GHOSTOS_ARCH_UNSUPPORTED_H
#define GHOSTOS_ARCH_UNSUPPORTED_H

#include <stdbool.h>
#include <stdint.h>

#define GHOSTOS_UNSUPPORTED_TABLE_FRAME_COUNT 6

void ghostos_unsupported_install_root(const uint64_t tables[GHOSTOS_UNSUPPORTED_TABLE_FRAME_COUNT], uint64_t physical_offset);
void ghostos_unsupported_invalidate_page(uint64_t address);
void ghostos_unsupported_invalidate_all(void);
void ghostos_unsupported_invalidate_range(uint64_t start, uint64_t length);
_Noreturn void ghostos_unsupported_enter_user(void);
void ghostos_unsupported_set_core_isolated(uint8_t cpu, bool isolated);
uint8_t ghostos_unsupported_current_cpu(void);
bool ghostos_unsupported_send_ipi(uint8_t target, uint8_t vector);
void ghostos_unsupported_end_of_interrupt(void);
void ghostos_unsupported_disable_interrupts(void);
void ghostos_unsupported_enable_interrupts(void);
void ghostos_unsupported_interrupts_init(void);
void ghostos_unsupported_halt(void);
void ghostos_unsupported_idle(void);

#endif
