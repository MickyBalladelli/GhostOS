CC ?= cc
AR ?= ar
PYTHON ?= python3
CFLAGS ?= -O2 -g
CPPFLAGS += -Ic/include
GHOSTOS_CFLAGS = -std=c11 -Wall -Wextra -Werror -pedantic
BUILD_DIR ?= build/c
SOURCES = c/src/dsm.c c/src/cxl.c c/src/fabric_lease.c c/src/kv_cache.c c/src/inference_journal.c c/src/inference_schedule.c c/src/inference.c c/src/config_diff.c c/src/config_capability.c c/src/config_cluster.c c/src/config_network.c c/src/config_parser.c c/src/reconfigure.c c/src/config_signature.c c/src/backup_crypto.c c/src/backup.c c/src/attestation.c c/src/confidential_fabric.c c/src/confidential_capability.c c/src/enclave.c c/src/remote_debug.c c/src/coredump.c c/src/gdb.c c/src/probes.c c/src/embedded_script.c c/src/wasm_script.c c/src/logd.c c/src/audit.c c/src/agent.c c/src/rms.c c/src/heal.c c/src/actors.c c/src/service_scale.c c/src/power_policy.c c/src/thermal.c c/src/posix_compat.c c/src/ras.c c/src/policy.c c/src/platform_io.c c/src/numa.c c/src/durability.c c/src/time_sync.c c/src/admission.c c/src/path_pattern.c c/src/status.c c/src/abi.c c/src/api_compat.c c/src/protocol.c c/src/boot_protocol.c c/src/address_space.c c/src/frame_allocator.c c/src/cow.c c/src/arch_aarch64.c c/src/arch_riscv64.c c/src/arch_unsupported.c c/src/cpu_topology.c c/src/arch.c c/src/boot_diagnostics.c c/src/boot_services.c c/src/capability.c c/src/console.c c/src/contention.c c/src/crash.c c/src/dlm.c c/src/dma.c c/src/dma_state.c c/src/driver_capabilities.c c/src/driver_resources.c c/src/hot_allocator.c c/src/invariants.c c/src/ipc.c c/src/keyboard.c c/src/keyboard_stub.c c/src/kernel.c c/src/litmus.c c/src/main.c c/src/micro_silo.c c/src/monitor.c c/src/mouse.c c/src/mouse_stub.c c/src/page_fault.c c/src/partition.c c/src/pci.c c/src/persistence.c c/src/persistence_records.c c/src/persona.c c/src/physical_storage.c c/src/power.c c/src/process.c c/src/quota.c c/src/random.c c/src/runtime.c c/src/saturation.c c/src/scheduler.c c/src/shell.c c/src/syscall.c c/src/task.c c/src/time.c c/src/tlb.c c/src/usb_keyboard.c c/src/usb_keyboard_controller.c c/src/usb_keyboard_stub.c c/src/watchdog.c c/src/webauthn.c c/src/vm_boot.c c/src/vm_clock.c c/src/vm_cluster.c
SOURCES += c/src/vm_apic.c c/src/vm_hpet.c c/src/vm_pit.c c/src/vm_input.c c/src/vm_ps2.c c/src/vm_driver_capabilities.c c/src/vm_guest.c c/src/vm_mac.c c/src/vm_power.c c/src/vm_interrupt_controller.c c/src/vm_virtio_queue.c c/src/vm_virtio.c c/src/vm_virtio_net.c c/src/vm_packet.c c/src/vm_serial.c c/src/vm_persistence.c c/src/vm_migration.c c/src/vm_snapshot_auth.c c/src/vm_dhcp.c c/src/vm_net.c c/src/vm_bios.c c/src/vm_replay.c c/src/vm_display.c c/src/vm_terminal_platform.c c/src/vm_terminal.c c/src/vm_e1000.c c/src/vm_nvme.c c/src/vm_ahci.c c/src/vm_segment.c c/src/vm_host_net.c c/src/vm_acceleration.c c/src/vm_disk_image.c c/src/vm_disk_management.c c/src/vm_control.c c/src/vm_passkey.c c/src/vm_passkey_assets.c c/src/vm_execution.c c/src/vm_uefi.c
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
c-test-binaries: $(BUILD_DIR)/dsm-contracts $(BUILD_DIR)/cxl-contracts $(BUILD_DIR)/fabric-lease-contracts $(BUILD_DIR)/kv-cache-contracts $(BUILD_DIR)/inference-journal-contracts $(BUILD_DIR)/inference-schedule-contracts $(BUILD_DIR)/inference-contracts $(BUILD_DIR)/config-diff-contracts $(BUILD_DIR)/config-capability-contracts $(BUILD_DIR)/config-cluster-contracts $(BUILD_DIR)/config-network-contracts $(BUILD_DIR)/config-service-contracts $(BUILD_DIR)/config-parser-contracts $(BUILD_DIR)/reconfigure-contracts $(BUILD_DIR)/config-signature-contracts $(BUILD_DIR)/backup-crypto-contracts $(BUILD_DIR)/backup-contracts $(BUILD_DIR)/attestation-contracts $(BUILD_DIR)/confidential-fabric-contracts $(BUILD_DIR)/confidential-capability-contracts $(BUILD_DIR)/enclave-contracts $(BUILD_DIR)/remote-debug-contracts $(BUILD_DIR)/coredump-contracts $(BUILD_DIR)/gdb-contracts $(BUILD_DIR)/probe-contracts $(BUILD_DIR)/embedded-script-contracts $(BUILD_DIR)/wasm-script-contracts $(BUILD_DIR)/logd-contracts $(BUILD_DIR)/audit-contracts $(BUILD_DIR)/agent-contracts $(BUILD_DIR)/rms-contracts $(BUILD_DIR)/heal-contracts $(BUILD_DIR)/actor-contracts $(BUILD_DIR)/ras-contracts $(BUILD_DIR)/policy-contracts $(BUILD_DIR)/platform-io-contracts $(BUILD_DIR)/numa-contracts $(BUILD_DIR)/time-sync-contracts $(BUILD_DIR)/admission-contracts $(BUILD_DIR)/path-pattern-contracts $(BUILD_DIR)/foundation-tests $(BUILD_DIR)/kernel-contracts $(BUILD_DIR)/kernel-frame-contracts $(BUILD_DIR)/kernel-ipc-contracts $(BUILD_DIR)/kernel-usb-keyboard-contracts $(BUILD_DIR)/kernel-watchdog-contracts

c-test-support: $(BUILD_DIR)/libghostos-test-support.a

c-vm-test-binaries: $(BUILD_DIR)/vm-control-contracts $(BUILD_DIR)/vm-disk-management-contracts $(BUILD_DIR)/vm-device-contracts $(BUILD_DIR)/vm-serial-contracts $(BUILD_DIR)/vm-io-contracts $(BUILD_DIR)/vm-storage-contracts

$(BUILD_DIR)/vm-storage-contracts: c/tests/vm_storage_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/vm-io-contracts: c/tests/vm_io_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

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

$(BUILD_DIR)/path-pattern-contracts: c/tests/path_pattern_contracts.c $(BUILD_DIR)/libghostos.a $(BUILD_DIR)/libghostos-test-support.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a $(BUILD_DIR)/libghostos-test-support.a -o $@

$(BUILD_DIR)/vm-disk-management-contracts: c/tests/vm_disk_management_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/admission-contracts: c/tests/admission_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/time-sync-contracts: c/tests/time_sync_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/vm-control-contracts: c/tests/vm_control_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/numa-contracts: c/tests/numa_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/platform-io-contracts: c/tests/platform_io_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/policy-contracts: c/tests/policy_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/ras-contracts: c/tests/ras_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/actor-contracts: c/tests/actor_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/heal-contracts: c/tests/heal_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/rms-contracts: c/tests/rms_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/agent-contracts: c/tests/agent_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/audit-contracts: c/tests/audit_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/logd-contracts: c/tests/logd_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/wasm-script-contracts: c/tests/wasm_script_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/embedded-script-contracts: c/tests/embedded_script_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/probe-contracts: c/tests/probe_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/gdb-contracts: c/tests/gdb_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/coredump-contracts: c/tests/coredump_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/remote-debug-contracts: c/tests/remote_debug_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/enclave-contracts: c/tests/enclave_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/confidential-capability-contracts: c/tests/confidential_capability_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/confidential-fabric-contracts: c/tests/confidential_fabric_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/attestation-contracts: c/tests/attestation_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/backup-contracts: c/tests/backup_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/backup-crypto-contracts: c/tests/backup_crypto_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/config-signature-contracts: c/tests/config_signature_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/reconfigure-contracts: c/tests/reconfigure_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/config-parser-contracts: c/tests/config_parser_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/config-service-contracts: c/tests/config_service_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/config-network-contracts: c/tests/config_network_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/config-cluster-contracts: c/tests/config_cluster_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/config-capability-contracts: c/tests/config_capability_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/config-diff-contracts: c/tests/config_diff_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/inference-contracts: c/tests/inference_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/inference-schedule-contracts: c/tests/inference_schedule_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/inference-journal-contracts: c/tests/inference_journal_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/kv-cache-contracts: c/tests/kv_cache_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/fabric-lease-contracts: c/tests/fabric_lease_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/cxl-contracts: c/tests/cxl_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@

$(BUILD_DIR)/dsm-contracts: c/tests/dsm_contracts.c $(BUILD_DIR)/libghostos.a $(HEADERS)
	$(CC) $(CPPFLAGS) $(CFLAGS) $(GHOSTOS_CFLAGS) $< $(BUILD_DIR)/libghostos.a -o $@
