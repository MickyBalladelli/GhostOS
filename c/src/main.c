#include "ghostos/main.h"

_Noreturn void ghostos_main_panic(ghostos_main_panic_fn panic_handler, void *context) {
    if (panic_handler) panic_handler(context);
    for (;;) { }
}

#if defined(GHOSTOS_KERNEL_IMAGE)
extern _Noreturn void kernel_entry(const ghostos_boot_info *boot_info);

__attribute__((section(".text._start"), used, noreturn))
void _start(const ghostos_boot_info *boot_info) {
    kernel_entry(boot_info);
}
#endif
