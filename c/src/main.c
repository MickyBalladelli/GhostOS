#include "ghostos/main.h"

_Noreturn void ghostos_main_enter(const ghostos_boot_info *boot_info,
                                  ghostos_main_entry_fn entry) {
    if (boot_info && entry) entry(boot_info);
    for (;;) { }
}

_Noreturn void ghostos_main_panic(ghostos_main_panic_fn panic_handler, void *context) {
    if (panic_handler) panic_handler(context);
    for (;;) { }
}
