#include "ghostos/usb_keyboard.h"

#define zero_bytes(bytes, length) __builtin_memset((bytes), 0, (length))
#define copy_bytes(destination, source, length) __builtin_memcpy((destination), (source), (length))

#if defined(GHOSTOS_USB_XHCI_ENABLED) && defined(__x86_64__)

#define TRB_COUNT 256u
#define EVENT_COUNT 256u
#define CONTEXT_BYTES 4096u
#define BUFFER_BYTES 4096u
#define TYPE_NORMAL 1u
#define TYPE_SETUP_STAGE 2u
#define TYPE_DATA_STAGE 3u
#define TYPE_STATUS_STAGE 4u
#define TYPE_ENABLE_SLOT 9u
#define TYPE_DISABLE_SLOT 10u
#define TYPE_ADDRESS_DEVICE 11u
#define TYPE_CONFIGURE_ENDPOINT 12u
#define TYPE_EVALUATE_CONTEXT 13u
#define TYPE_TRANSFER_EVENT 32u
#define TYPE_COMMAND_COMPLETION 33u
#define COMPLETION_SUCCESS 1u
#define COMPLETION_SHORT_PACKET 13u

typedef struct __attribute__((aligned(16))) {
    uint32_t parameter_low, parameter_high, status, control;
} xhci_trb;

typedef struct {
    volatile xhci_trb *address;
    size_t index;
    uint32_t cycle;
} producer_ring;

typedef struct {
    xhci_trb trb;
} xhci_event;

typedef struct {
    volatile uint8_t *operational;
    volatile uint8_t *runtime;
    volatile uint8_t *doorbells;
    size_t context_size;
    uint8_t max_ports;
    size_t event_index;
    uint32_t event_cycle;
    producer_ring command, ep0, interrupt;
    uint8_t slot_id, endpoint_id;
    uint16_t interrupt_packet_size;
    bool ready;
} xhci_keyboard;

static xhci_trb command_ring[TRB_COUNT] __attribute__((aligned(4096)));
static xhci_trb event_ring[EVENT_COUNT] __attribute__((aligned(4096)));
static xhci_trb ep0_ring[TRB_COUNT] __attribute__((aligned(4096)));
static xhci_trb interrupt_ring[TRB_COUNT] __attribute__((aligned(4096)));
static uint64_t dcbaa[512] __attribute__((aligned(4096)));
static uint64_t erst[512] __attribute__((aligned(4096)));
static uint8_t device_context[CONTEXT_BYTES] __attribute__((aligned(4096)));
static uint8_t input_context[CONTEXT_BYTES] __attribute__((aligned(4096)));
static uint8_t control_buffer[BUFFER_BYTES] __attribute__((aligned(4096)));
static uint8_t report_buffer[BUFFER_BYTES] __attribute__((aligned(4096)));
static xhci_keyboard controller;
static ghostos_usb_keyboard_state keyboard_state;

static xhci_trb make_trb(uint64_t parameter, uint32_t status, uint32_t control) {
    return (xhci_trb){(uint32_t)parameter, (uint32_t)(parameter >> 32), status, control};
}

static uint32_t trb_type(xhci_trb trb) { return (trb.control >> 10) & 0x3f; }
static uint8_t completion_code(xhci_trb trb) { return (uint8_t)(trb.status >> 24); }
static bool completion_ok(uint8_t code) {
    return code == COMPLETION_SUCCESS || code == COMPLETION_SHORT_PACKET;
}

static uint8_t read8(volatile uint8_t *base, size_t offset) {
    return *(volatile uint8_t *)(base + offset);
}

static uint32_t read32(volatile uint8_t *base, size_t offset) {
    return *(volatile uint32_t *)(base + offset);
}

static void write32(volatile uint8_t *base, size_t offset, uint32_t value) {
    *(volatile uint32_t *)(base + offset) = value;
}

static void write64(volatile uint8_t *base, size_t offset, uint64_t value) {
    *(volatile uint64_t *)(base + offset) = value;
}

static void pause_cpu(void) {
    __asm__ volatile("pause");
}

static void ring_init(producer_ring *ring, volatile xhci_trb *address) {
    ring->address = address;
    ring->index = 0;
    ring->cycle = 1;
}

static void ring_push(producer_ring *ring, xhci_trb trb) {
    trb.control = (trb.control & ~UINT32_C(1)) | ring->cycle;
    ring->address[ring->index] = trb;
    ++ring->index;
    if (ring->index == TRB_COUNT - 1) {
        uint64_t address = (uint64_t)(uintptr_t)ring->address;
        ring->address[TRB_COUNT - 1] = make_trb(address, 0,
            (6u << 10) | (1u << 1) | ring->cycle);
        ring->index = 0;
        ring->cycle ^= 1;
    }
}

static uint64_t ring_dequeue(const producer_ring *ring) {
    return (uint64_t)(uintptr_t)&ring->address[ring->index] | ring->cycle;
}

static void clear_input_context(void) {
    zero_bytes(input_context, CONTEXT_BYTES);
}

static void context_write(size_t context, size_t dword, uint32_t value) {
    *(volatile uint32_t *)(input_context + context + dword * 4) = value;
}

static uint32_t context_read(size_t context, size_t dword) {
    return *(volatile uint32_t *)(input_context + context + dword * 4);
}

static void legacy_handoff(volatile uint8_t *capability, uint32_t hcc) {
    size_t offset = (size_t)((hcc >> 16) * 4u);
    for (size_t count = 0; count < 64; ++count) {
        if (offset == 0) return;
        uint32_t header = read32(capability, offset);
        if ((header & 0xff) == 1) {
            write32(capability, offset, header | (1u << 24));
            for (size_t wait = 0; wait < 1000000; ++wait) {
                if ((read32(capability, offset) & (1u << 16)) == 0) return;
                pause_cpu();
            }
            return;
        }
        size_t next = (header >> 8) & 0xff;
        if (next == 0) return;
        offset += next * 4;
    }
}

static bool wait_for(volatile uint8_t *base, size_t offset, uint32_t mask, uint32_t expected) {
    for (size_t count = 0; count < 10000000; ++count) {
        if ((read32(base, offset) & mask) == expected) return true;
        pause_cpu();
    }
    return false;
}

static bool next_event(xhci_event *event) {
    volatile xhci_trb *ring = event_ring;
    xhci_trb trb = ring[controller.event_index];
    if ((trb.control & 1) != controller.event_cycle) return false;
    ++controller.event_index;
    if (controller.event_index == EVENT_COUNT) {
        controller.event_index = 0;
        controller.event_cycle ^= 1;
    }
    uint64_t dequeue = (uint64_t)(uintptr_t)&ring[controller.event_index];
    write64(controller.runtime + 0x20, 0x18, dequeue | (1u << 3));
    event->trb = trb;
    return true;
}

static bool wait_event(uint32_t type, xhci_event *event) {
    for (size_t count = 0; count < 10000000; ++count) {
        xhci_event current;
        if (next_event(&current) && trb_type(current.trb) == type) {
            *event = current;
            return true;
        }
        pause_cpu();
    }
    return false;
}

static void ring_doorbell(uint8_t endpoint_id) {
    write32(controller.doorbells, (size_t)controller.slot_id * 4, endpoint_id);
}

static bool command(xhci_trb trb, xhci_event *event) {
    ring_push(&controller.command, trb);
    write32(controller.doorbells, 0, 0);
    if (!wait_event(TYPE_COMMAND_COMPLETION, event)) return false;
    return completion_code(event->trb) == COMPLETION_SUCCESS;
}

static uint64_t setup_packet(uint8_t request_type, uint8_t request, uint16_t value,
    uint16_t index, uint16_t length) {
    return (uint64_t)request_type | (uint64_t)request << 8 | (uint64_t)value << 16 |
        (uint64_t)index << 32 | (uint64_t)length << 48;
}

static bool control_in(uint8_t request_type, uint8_t request, uint16_t value,
    uint16_t index, uint16_t length, size_t *actual_length) {
    zero_bytes(control_buffer, BUFFER_BYTES);
    uint64_t setup = setup_packet(request_type, request, value, index, length);
    ring_push(&controller.ep0, make_trb(setup, 8,
        (TYPE_SETUP_STAGE << 10) | (1u << 6) | (3u << 16)));
    ring_push(&controller.ep0, make_trb((uint64_t)(uintptr_t)control_buffer, length,
        (TYPE_DATA_STAGE << 10) | (1u << 16)));
    ring_push(&controller.ep0, make_trb(0, 0,
        (TYPE_STATUS_STAGE << 10) | (1u << 5)));
    ring_doorbell(1);
    xhci_event event;
    if (!wait_event(TYPE_TRANSFER_EVENT, &event) || !completion_ok(completion_code(event.trb)))
        return false;
    *actual_length = (size_t)length - (event.trb.status & 0x00ffffff);
    return true;
}

static bool control_out(uint8_t request_type, uint8_t request, uint16_t value, uint16_t index) {
    uint64_t setup = setup_packet(request_type, request, value, index, 0);
    ring_push(&controller.ep0, make_trb(setup, 8, (TYPE_SETUP_STAGE << 10) | (1u << 6)));
    ring_push(&controller.ep0, make_trb(0, 0,
        (TYPE_STATUS_STAGE << 10) | (1u << 16) | (1u << 5)));
    ring_doorbell(1);
    xhci_event event;
    return wait_event(TYPE_TRANSFER_EVENT, &event) && completion_ok(completion_code(event.trb));
}

static void prepare_address_context(uint8_t root_port, uint8_t speed, uint16_t max_packet) {
    clear_input_context();
    context_write(0, 1, (1u << 0) | (1u << 1));
    size_t slot = controller.context_size;
    context_write(slot, 0, ((uint32_t)speed << 20) | (1u << 27));
    context_write(slot, 1, (uint32_t)root_port << 16);
    size_t endpoint = controller.context_size * 2;
    context_write(endpoint, 1, (3u << 1) | (4u << 3) | ((uint32_t)max_packet << 16));
    uint64_t ring = ring_dequeue(&controller.ep0);
    context_write(endpoint, 2, (uint32_t)ring);
    context_write(endpoint, 3, (uint32_t)(ring >> 32));
    context_write(endpoint, 4, 8);
}

static void prepare_ep0_update(uint16_t max_packet) {
    clear_input_context();
    context_write(0, 1, 1u << 1);
    size_t endpoint = controller.context_size * 2;
    context_write(endpoint, 1, (3u << 1) | (4u << 3) | ((uint32_t)max_packet << 16));
    uint64_t ring = ring_dequeue(&controller.ep0);
    context_write(endpoint, 2, (uint32_t)ring);
    context_write(endpoint, 3, (uint32_t)(ring >> 32));
    context_write(endpoint, 4, 8);
}

static bool configure_interrupt_endpoint(uint8_t endpoint, uint16_t packet_size, uint8_t interval) {
    uint8_t number = endpoint & 0x0f;
    if (number == 0 || (endpoint & 0x80) == 0) return false;
    controller.endpoint_id = (uint8_t)(number * 2u + 1u);
    controller.interrupt_packet_size = packet_size;
    clear_input_context();
    context_write(0, 1, (1u << 0) | (1u << controller.endpoint_id));
    size_t slot = controller.context_size;
    copy_bytes(input_context + slot, device_context, controller.context_size);
    uint32_t current_slot = context_read(slot, 0);
    context_write(slot, 0, (current_slot & ~(0x1fu << 27)) |
        ((uint32_t)controller.endpoint_id << 27));
    size_t context = controller.context_size * ((size_t)controller.endpoint_id + 1);
    uint32_t normalized_interval = (interval ? interval - 1u : 0u);
    if (normalized_interval > 15) normalized_interval = 15;
    context_write(context, 0, normalized_interval << 16);
    context_write(context, 1, (3u << 1) | (7u << 3) | ((uint32_t)packet_size << 16));
    uint64_t ring = (uint64_t)(uintptr_t)interrupt_ring | 1;
    context_write(context, 2, (uint32_t)ring);
    context_write(context, 3, (uint32_t)(ring >> 32));
    context_write(context, 4, packet_size | ((uint32_t)packet_size << 16));
    xhci_event event;
    return command(make_trb((uint64_t)(uintptr_t)input_context, 0,
        (TYPE_CONFIGURE_ENDPOINT << 10) | ((uint32_t)controller.slot_id << 24)), &event);
}

static bool queue_report(void) {
    uint32_t length = controller.interrupt_packet_size > 8 ?
        controller.interrupt_packet_size : 8;
    ring_push(&controller.interrupt, make_trb((uint64_t)(uintptr_t)report_buffer, length,
        (TYPE_NORMAL << 10) | (1u << 5) | (1u << 2)));
    ring_doorbell(controller.endpoint_id);
    return true;
}

static bool configure_device(uint8_t root_port, uint8_t speed) {
    xhci_event event;
    if (!command(make_trb(0, 0, TYPE_ENABLE_SLOT << 10), &event)) return false;
    controller.slot_id = (uint8_t)(event.trb.control >> 24);
    if (controller.slot_id == 0) return false;
    dcbaa[controller.slot_id] = (uint64_t)(uintptr_t)device_context;
    zero_bytes(input_context, CONTEXT_BYTES);
    uint16_t max_packet = speed == 3 ? 64 : (speed >= 4 ? 512 : 8);
    prepare_address_context(root_port, speed, max_packet);
    if (!command(make_trb((uint64_t)(uintptr_t)input_context, 0,
            (TYPE_ADDRESS_DEVICE << 10) | ((uint32_t)controller.slot_id << 24)), &event))
        return false;

    size_t descriptor_length = 0;
    if (!control_in(0x80, 6, 0x0100, 0, 18, &descriptor_length) || descriptor_length < 8)
        return false;
    uint16_t actual_max_packet = control_buffer[7];
    if (actual_max_packet != 0 && actual_max_packet != max_packet) {
        prepare_ep0_update(actual_max_packet);
        if (!command(make_trb((uint64_t)(uintptr_t)input_context, 0,
                (TYPE_EVALUATE_CONTEXT << 10) | ((uint32_t)controller.slot_id << 24)), &event))
            return false;
    }

    size_t header_length = 0;
    if (!control_in(0x80, 6, 0x0200, 0, 9, &header_length) || header_length < 9)
        return false;
    size_t total_length = (size_t)control_buffer[2] | ((size_t)control_buffer[3] << 8);
    uint8_t configuration_value = control_buffer[5];
    size_t fetched = 0;
    uint16_t requested_length = (uint16_t)(total_length < BUFFER_BYTES ? total_length : BUFFER_BYTES);
    if (!control_in(0x80, 6, 0x0200, 0, requested_length, &fetched)) return false;
    uint8_t interface_number = 0, endpoint = 0, interval = 0;
    uint16_t packet_size = 0;
    if (!ghostos_usb_keyboard_find_descriptor(control_buffer, fetched,
            &interface_number, &endpoint, &packet_size, &interval))
        return false;
    return control_out(0x00, 9, configuration_value, 0) &&
        control_out(0x21, 11, 0, interface_number) &&
        configure_interrupt_endpoint(endpoint, packet_size, interval) && queue_report();
}

static void release_slot(void) {
    if (controller.slot_id == 0) return;
    uint8_t slot = controller.slot_id;
    xhci_event event;
    (void)command(make_trb(0, 0, (TYPE_DISABLE_SLOT << 10) | ((uint32_t)slot << 24)), &event);
    dcbaa[slot] = 0;
    controller.slot_id = 0;
}

static bool attach_keyboard(void) {
    for (uint16_t port = 0; port < controller.max_ports; ++port) {
        size_t reg = 0x400 + (size_t)port * 0x10;
        uint32_t status = read32(controller.operational, reg);
        if ((status & 1) == 0) continue;
        if ((status & (1u << 9)) == 0) write32(controller.operational, reg, status | (1u << 9));
        write32(controller.operational, reg, status | (1u << 4));
        bool reset_done = false;
        for (size_t count = 0; count < 2000000; ++count) {
            status = read32(controller.operational, reg);
            if ((status & (1u << 4)) == 0 && (status & (1u << 1)) != 0) {
                reset_done = true;
                break;
            }
            pause_cpu();
        }
        if (!reset_done) continue;
        uint8_t speed = (uint8_t)((status >> 10) & 0xf);
        if (configure_device((uint8_t)(port + 1), speed)) return true;
        release_slot();
    }
    return false;
}

static bool initialize_controller(uint64_t mmio) {
    zero_bytes(command_ring, sizeof(command_ring));
    zero_bytes(event_ring, sizeof(event_ring));
    zero_bytes(ep0_ring, sizeof(ep0_ring));
    zero_bytes(interrupt_ring, sizeof(interrupt_ring));
    zero_bytes(dcbaa, sizeof(dcbaa));
    zero_bytes(erst, sizeof(erst));
    zero_bytes(device_context, sizeof(device_context));
    zero_bytes(input_context, sizeof(input_context));
    zero_bytes(control_buffer, sizeof(control_buffer));
    zero_bytes(report_buffer, sizeof(report_buffer));
    controller = (xhci_keyboard){0};
    ghostos_usb_keyboard_init(&keyboard_state);

    volatile uint8_t *capability = (volatile uint8_t *)(uintptr_t)mmio;
    uint8_t capability_length = read8(capability, 0);
    uint32_t parameters1 = read32(capability, 0x04);
    uint32_t parameters2 = read32(capability, 0x08);
    uint32_t scratchpads = (((parameters2 >> 21) & 0x1f) << 5) |
        ((parameters2 >> 27) & 0x1f);
    if (scratchpads != 0) return false;
    uint32_t hcc = read32(capability, 0x10);
    legacy_handoff(capability, hcc);
    size_t doorbell_offset = read32(capability, 0x14) & ~3u;
    size_t runtime_offset = read32(capability, 0x18) & ~0x1fu;
    controller.operational = capability + capability_length;
    controller.runtime = capability + runtime_offset;
    controller.doorbells = capability + doorbell_offset;
    controller.context_size = (hcc & (1u << 2)) ? 64 : 32;
    controller.max_ports = (uint8_t)(parameters1 >> 24);
    controller.event_index = 0;
    controller.event_cycle = 1;
    ring_init(&controller.command, command_ring);
    ring_init(&controller.ep0, ep0_ring);
    ring_init(&controller.interrupt, interrupt_ring);

    uint32_t command_reg = read32(controller.operational, 0x00) & ~1u;
    write32(controller.operational, 0x00, command_reg);
    if (!wait_for(controller.operational, 0x04, 1, 1)) return false;
    write32(controller.operational, 0x00, command_reg | (1u << 1));
    if (!wait_for(controller.operational, 0x00, 1u << 1, 0) ||
        !wait_for(controller.operational, 0x04, 1u << 11, 0)) return false;
    if ((read32(controller.operational, 0x08) & 1) == 0) return false;
    write64(controller.operational, 0x18, (uint64_t)(uintptr_t)command_ring | 1);
    write64(controller.operational, 0x30, (uint64_t)(uintptr_t)dcbaa);
    write32(controller.operational, 0x38, (read32(controller.operational, 0x38) & ~0xffu) | 1);
    erst[0] = (uint64_t)(uintptr_t)event_ring;
    erst[1] = EVENT_COUNT;
    volatile uint8_t *interrupter = controller.runtime + 0x20;
    write32(interrupter, 0x08, 1);
    write64(interrupter, 0x10, (uint64_t)(uintptr_t)erst);
    write64(interrupter, 0x18, (uint64_t)(uintptr_t)event_ring);
    write32(controller.operational, 0x04, UINT32_MAX);
    write32(controller.operational, 0x00, command_reg | 1);
    if (!wait_for(controller.operational, 0x04, 1, 0)) return false;
    return attach_keyboard();
}

bool ghostos_usb_keyboard_controller_init(uint64_t mmio) {
    if (mmio == 0) return false;
    controller.ready = initialize_controller(mmio);
    return controller.ready;
}

bool ghostos_usb_keyboard_controller_poll(uint8_t report[8]) {
    if (!controller.ready || !report) return false;
    xhci_event event;
    if (!next_event(&event) || trb_type(event.trb) != TYPE_TRANSFER_EVENT ||
        !completion_ok(completion_code(event.trb)) ||
        (uint8_t)(event.trb.control >> 24) != controller.slot_id)
        return false;
    for (size_t index = 0; index < 8; ++index) report[index] = report_buffer[index];
    (void)queue_report();
    return true;
}

#else
bool ghostos_usb_keyboard_controller_init(uint64_t mmio) {
    (void)mmio;
    return false;
}

bool ghostos_usb_keyboard_controller_poll(uint8_t report[8]) {
    (void)report;
    return false;
}
#endif
