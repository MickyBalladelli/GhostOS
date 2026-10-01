#ifndef GHOSTOS_MAIN_H
#define GHOSTOS_MAIN_H

#include "ghostos/boot_protocol.h"

typedef void (*ghostos_main_panic_fn)(void *context);

_Noreturn void ghostos_main_panic(ghostos_main_panic_fn panic_handler, void *context);

#if defined(GHOSTOS_KERNEL_IMAGE)
_Noreturn void _start(const ghostos_boot_info *boot_info);
#endif

#endif
