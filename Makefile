CC ?= cc
AR ?= ar
PYTHON ?= python3
CFLAGS ?= -O2 -g
CPPFLAGS += -Ic/include
GHOSTOS_CFLAGS = -std=c11 -Wall -Wextra -Werror -pedantic
BUILD_DIR ?= build/c
SOURCES = c/src/status.c c/src/abi.c c/src/api_compat.c c/src/protocol.c c/src/boot_protocol.c c/src/address_space.c c/src/frame_allocator.c c/src/cow.c c/src/arch_aarch64.c c/src/arch_riscv64.c c/src/arch_unsupported.c c/src/cpu_topology.c c/src/arch.c c/src/boot_diagnostics.c c/src/boot_services.c c/src/capability.c c/src/console.c c/src/contention.c c/src/crash.c c/src/dlm.c c/src/dma.c c/src/driver_capabilities.c c/src/hot_allocator.c c/src/invariants.c c/src/ipc.c c/src/keyboard.c c/src/keyboard_stub.c c/src/kernel.c c/src/litmus.c c/src/main.c c/src/micro_silo.c c/src/monitor.c c/src/mouse.c c/src/mouse_stub.c c/src/page_fault.c c/src/partition.c c/src/pci.c c/src/persistence.c c/src/persona.c c/src/physical_storage.c c/src/power.c c/src/process.c c/src/quota.c c/src/random.c c/src/runtime.c c/src/saturation.c c/src/scheduler.c c/src/shell.c c/src/syscall.c c/src/task.c c/src/time.c c/src/tlb.c c/src/usb_keyboard.c c/src/usb_keyboard_controller.c c/src/usb_keyboard_stub.c c/src/watchdog.c c/src/webauthn.c c/src/vm_boot.c c/src/vm_clock.c c/src/vm_cluster.c
SOURCES += c/src/vm_apic.c c/src/vm_hpet.c c/src/vm_pit.c c/src/vm_input.c c/src/vm_ps2.c c/src/vm_driver_capabilities.c c/src/vm_guest.c c/src/vm_mac.c c/src/vm_power.c c/src/vm_interrupt_controller.c c/src/vm_virtio_queue.c c/src/vm_virtio.c c/src/vm_serial.c c/src/vm_persistence.c
OBJECTS = $(patsubst c/src/%.c,$(BUILD_DIR)/%.o,$(SOURCES))
HEADERS = $(wildcard c/include/ghostos/*.h)
TEST_SUPPORT_OBJECTS = $(BUILD_DIR)/test_support/property.o

.PHONY: all c-library generate-c-abi c-test-support c-test-binaries c-vm-test-binaries
all: c-library

c-library: $(BUILD_DIR)/libghostos.a

generate-c-abi:
	$(PYTHON) tools/generate_c_abi.py

$(BUILD_DIR)/%.o: c/src/%.c $(HEADERS)
	mkdir -p $(BUILD_DIR)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) -ffreestanding -c $< -o $@

$(BUILD_DIR)/libghostos.a: $(OBJECTS)
	$(AR) rcs $@ $(OBJECTS)

# Explicit opt-in target builds tests but does not execute them.
c-test-binaries: $(BUILD_DIR)/foundation-tests $(BUILD_DIR)/kernel-contracts $(BUILD_DIR)/kernel-frame-contracts $(BUILD_DIR)/kernel-ipc-contracts $(BUILD_DIR)/kernel-usb-keyboard-contracts $(BUILD_DIR)/kernel-watchdog-contracts

c-test-support: $(BUILD_DIR)/libghostos-test-support.a

c-vm-test-binaries: $(BUILD_DIR)/vm-device-contracts $(BUILD_DIR)/vm-serial-contracts

$(BUILD_DIR)/vm-serial-contracts: c/tests/vm_serial_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/vm-device-contracts: c/tests/vm_device_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/test_support/%.o: c/test_support/%.c $(HEADERS)
	mkdir -p $(BUILD_DIR)/test_support
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) -c $< -o $@

$(BUILD_DIR)/libghostos-test-support.a: $(TEST_SUPPORT_OBJECTS)
	$(AR) rcs $@ $(TEST_SUPPORT_OBJECTS)

$(BUILD_DIR)/foundation-tests: c/tests/foundation.c $(BUILD_DIR)/libghostos.a $(BUILD_DIR)/libghostos-test-support.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a $(BUILD_DIR)/libghostos-test-support.a -o $@

$(BUILD_DIR)/kernel-contracts: c/tests/kernel_contracts.c $(BUILD_DIR)/libghostos.a $(BUILD_DIR)/libghostos-test-support.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a $(BUILD_DIR)/libghostos-test-support.a -o $@

$(BUILD_DIR)/kernel-frame-contracts: c/tests/kernel_frame_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/kernel-ipc-contracts: c/tests/kernel_ipc_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/kernel-usb-keyboard-contracts: c/tests/kernel_usb_keyboard_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/kernel-watchdog-contracts: c/tests/kernel_watchdog_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@
