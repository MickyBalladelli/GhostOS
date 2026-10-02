#include "ghostos/vm_net.h"

uint64_t ghostos_vm_net_align_up(uint64_t value, uint64_t alignment) {
    return (value + alignment - 1) & ~(alignment - 1);
}
