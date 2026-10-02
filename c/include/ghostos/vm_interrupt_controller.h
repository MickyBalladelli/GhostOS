#ifndef GHOSTOS_VM_INTERRUPT_CONTROLLER_H
#define GHOSTOS_VM_INTERRUPT_CONTROLLER_H

#include <stdbool.h>
#include <stdint.h>

typedef struct ghostos_vm_interrupt_controller ghostos_vm_interrupt_controller;

typedef struct {
    uint16_t selector;
    uint8_t type_attr;
    uint64_t offset;
    uint8_t ist;
} ghostos_vm_idt_gate;

ghostos_vm_interrupt_controller *ghostos_vm_interrupt_controller_new(void);
void ghostos_vm_interrupt_controller_free(ghostos_vm_interrupt_controller *controller);
bool ghostos_vm_idt_gate_decode(const uint8_t raw[16], ghostos_vm_idt_gate *out);
bool ghostos_vm_idt_gate_present(const ghostos_vm_idt_gate *gate);
uint8_t ghostos_vm_idt_gate_dpl(const ghostos_vm_idt_gate *gate);
void ghostos_vm_interrupt_controller_init(ghostos_vm_interrupt_controller *controller);
void ghostos_vm_interrupt_controller_set_idt(ghostos_vm_interrupt_controller *controller,
    uint64_t base, uint16_t limit);
bool ghostos_vm_interrupt_controller_idt_entry(const ghostos_vm_interrupt_controller *controller,
    uint8_t vector, uint64_t *address);
void ghostos_vm_interrupt_controller_map_irq(ghostos_vm_interrupt_controller *controller,
    uint8_t irq, uint8_t vector);
bool ghostos_vm_interrupt_controller_handle_irq(const ghostos_vm_interrupt_controller *controller,
    uint8_t irq, uint8_t *vector);
void ghostos_vm_interrupt_controller_remap_pic(ghostos_vm_interrupt_controller *controller);
void ghostos_vm_interrupt_controller_reset(ghostos_vm_interrupt_controller *controller);
bool ghostos_vm_interrupt_controller_is_mapped(const ghostos_vm_interrupt_controller *controller);
uint16_t ghostos_vm_interrupt_controller_idt_limit(const ghostos_vm_interrupt_controller *controller);
uint64_t ghostos_vm_interrupt_controller_idt_base(const ghostos_vm_interrupt_controller *controller);
bool ghostos_vm_interrupt_controller_route_at(const ghostos_vm_interrupt_controller *controller,
    uint16_t index, uint8_t *irq, uint64_t *vector);

#endif
