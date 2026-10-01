#ifndef GHOSTOS_MAIN_H
#define GHOSTOS_MAIN_H

#include "ghostos/boot_protocol.h"

typedef void (*ghostos_main_entry_fn)(const ghostos_boot_info *boot_info);
typedef void (*ghostos_main_panic_fn)(void *context);

_Noreturn void ghostos_main_enter(const ghostos_boot_info *boot_info,
                                  ghostos_main_entry_fn entry);
_Noreturn void ghostos_main_panic(ghostos_main_panic_fn panic_handler, void *context);

#endif
