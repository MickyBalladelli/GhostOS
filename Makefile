CC ?= cc
AR ?= ar
PYTHON ?= python3
CFLAGS ?= -O2 -g
CPPFLAGS += -Ic/include
GHOSTOS_CFLAGS = -std=c11 -Wall -Wextra -Werror -pedantic
BUILD_DIR ?= build/c
SOURCES = c/src/status.c c/src/abi.c c/src/api_compat.c c/src/protocol.c c/src/boot_protocol.c c/src/address_space.c c/src/frame_allocator.c c/src/cow.c c/src/arch_aarch64.c c/src/arch_riscv64.c c/src/arch_unsupported.c c/src/cpu_topology.c c/src/arch.c c/src/boot_diagnostics.c c/src/boot_services.c c/src/capability.c c/src/console.c c/src/contention.c c/src/crash.c c/src/dlm.c c/src/dma.c c/src/driver_capabilities.c c/src/hot_allocator.c c/src/invariants.c c/src/ipc.c c/src/keyboard.c c/src/keyboard_stub.c c/src/kernel.c c/src/litmus.c c/src/main.c c/src/micro_silo.c c/src/monitor.c c/src/mouse.c c/src/mouse_stub.c c/src/page_fault.c c/src/partition.c c/src/pci.c c/src/persistence.c c/src/persona.c c/src/physical_storage.c c/src/power.c c/src/process.c c/src/quota.c c/src/random.c c/src/runtime.c c/src/saturation.c c/src/scheduler.c c/src/shell.c
OBJECTS = $(patsubst c/src/%.c,$(BUILD_DIR)/%.o,$(SOURCES))
HEADERS = $(wildcard c/include/ghostos/*.h)
TEST_SUPPORT_OBJECTS = $(BUILD_DIR)/test_support/property.o

.PHONY: all c-library generate-c-abi c-test-support c-test-binaries
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
c-test-binaries: $(BUILD_DIR)/foundation-tests

c-test-support: $(BUILD_DIR)/libghostos-test-support.a

$(BUILD_DIR)/test_support/%.o: c/test_support/%.c $(HEADERS)
	mkdir -p $(BUILD_DIR)/test_support
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) -c $< -o $@

$(BUILD_DIR)/libghostos-test-support.a: $(TEST_SUPPORT_OBJECTS)
	$(AR) rcs $@ $(TEST_SUPPORT_OBJECTS)

$(BUILD_DIR)/foundation-tests: c/tests/foundation.c $(BUILD_DIR)/libghostos.a $(BUILD_DIR)/libghostos-test-support.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a $(BUILD_DIR)/libghostos-test-support.a -o $@
