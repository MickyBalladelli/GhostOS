CC ?= cc
AR ?= ar
PYTHON ?= python3
CFLAGS ?= -O2 -g
CPPFLAGS += -Ic/include
GHOSTOS_CFLAGS = -std=c11 -Wall -Wextra -Werror -pedantic
BUILD_DIR ?= build/c
SOURCES = c/src/status.c c/src/abi.c c/src/api_compat.c c/src/protocol.c c/src/boot_protocol.c
OBJECTS = $(patsubst c/src/%.c,$(BUILD_DIR)/%.o,$(SOURCES))
HEADERS = $(wildcard c/include/ghostos/*.h)

.PHONY: all c-library generate-c-abi c-test-binaries
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

$(BUILD_DIR)/foundation-tests: c/tests/foundation.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@
