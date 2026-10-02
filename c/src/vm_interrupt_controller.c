#include "ghostos/vm_interrupt_controller.h"
#include <stdlib.h>

struct ghostos_vm_interrupt_controller {
    uint64_t idt_base;
    uint16_t idt_limit;
    uint64_t irq_vectors[256];
    bool irq_present[256];
    bool pic_mapped;
};

ghostos_vm_interrupt_controller *ghostos_vm_interrupt_controller_new(void) {
    return calloc(1, sizeof(ghostos_vm_interrupt_controller));
}

void ghostos_vm_interrupt_controller_free(ghostos_vm_interrupt_controller *controller) {
    free(controller);
}

bool ghostos_vm_idt_gate_decode(const uint8_t raw[16], ghostos_vm_idt_gate *out) {
    if (!raw || !out) return false;
    uint64_t offset_low = (uint64_t)raw[0] | ((uint64_t)raw[1] << 8);
    uint64_t offset_mid = (uint64_t)raw[6] | ((uint64_t)raw[7] << 8);
    uint64_t offset_high = (uint64_t)raw[8] | ((uint64_t)raw[9] << 8) |
        ((uint64_t)raw[10] << 16) | ((uint64_t)raw[11] << 24);
    *out = (ghostos_vm_idt_gate){
        (uint16_t)(raw[2] | ((uint16_t)raw[3] << 8)), raw[5],
        offset_low | (offset_mid << 16) | (offset_high << 32),
        (uint8_t)(raw[4] & 0x07u)
    };
    return true;
}

bool ghostos_vm_idt_gate_present(const ghostos_vm_idt_gate *gate) {
    return gate && (gate->type_attr & 0x80u) != 0;
}

uint8_t ghostos_vm_idt_gate_dpl(const ghostos_vm_idt_gate *gate) {
    return gate ? (uint8_t)((gate->type_attr >> 5) & 0x03u) : 0;
}

void ghostos_vm_interrupt_controller_init(ghostos_vm_interrupt_controller *controller) {
    if (!controller) return;
    controller->idt_base = UINT64_C(0x1000);
    controller->idt_limit = 0;
    controller->pic_mapped = false;
}

void ghostos_vm_interrupt_controller_set_idt(ghostos_vm_interrupt_controller *controller,
    uint64_t base, uint16_t limit) {
    if (!controller) return;
    controller->idt_base = base;
    controller->idt_limit = limit;
}

bool ghostos_vm_interrupt_controller_idt_entry(const ghostos_vm_interrupt_controller *controller,
    uint8_t vector, uint64_t *address) {
    if (!controller || controller->idt_base == 0 ||
        !address || ((uint32_t)vector * 16u + 16u) > ((uint32_t)controller->idt_limit + 1u)) return false;
    *address = controller->idt_base + (uint64_t)vector * 16u;
    return true;
}

void ghostos_vm_interrupt_controller_map_irq(ghostos_vm_interrupt_controller *controller,
    uint8_t irq, uint8_t vector) {
    if (!controller) return;
    controller->irq_vectors[irq] = vector;
    controller->irq_present[irq] = true;
}

bool ghostos_vm_interrupt_controller_handle_irq(const ghostos_vm_interrupt_controller *controller,
    uint8_t irq, uint8_t *vector) {
    if (!controller || !vector || !controller->irq_present[irq]) return false;
    *vector = (uint8_t)controller->irq_vectors[irq];
    return true;
}

void ghostos_vm_interrupt_controller_remap_pic(ghostos_vm_interrupt_controller *controller) {
    if (controller) controller->pic_mapped = true;
}

void ghostos_vm_interrupt_controller_reset(ghostos_vm_interrupt_controller *controller) {
    if (!controller) return;
    for (size_t irq = 0; irq < 256; ++irq) controller->irq_present[irq] = false;
    controller->pic_mapped = false;
}

bool ghostos_vm_interrupt_controller_is_mapped(const ghostos_vm_interrupt_controller *controller) {
    return controller && controller->pic_mapped;
}

uint16_t ghostos_vm_interrupt_controller_idt_limit(const ghostos_vm_interrupt_controller *controller) {
    return controller ? controller->idt_limit : 0;
}

uint64_t ghostos_vm_interrupt_controller_idt_base(const ghostos_vm_interrupt_controller *controller) {
    return controller ? controller->idt_base : 0;
}

bool ghostos_vm_interrupt_controller_route_at(const ghostos_vm_interrupt_controller *controller,
    uint16_t index, uint8_t *irq, uint64_t *vector) {
    if (!controller || !irq || !vector || index >= 256) return false;
    uint16_t found = 0;
    for (uint16_t candidate = 0; candidate < 256; ++candidate) {
        if (!controller->irq_present[candidate]) continue;
        if (found++ != index) continue;
        *irq = (uint8_t)candidate;
        *vector = controller->irq_vectors[candidate];
        return true;
    }
    return false;
}
