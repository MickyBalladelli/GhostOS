#include "ghostos/vm_serial.h"
#include "ghostos/vm_apic.h"

#include <assert.h>
#include <stdio.h>
#include <string.h>

/* The fourteen existing serial.rs cases, using the native C device API. */
static ghostos_vm_serial *new_serial(void) {
    ghostos_vm_serial *serial = ghostos_vm_serial_new(0x3f8, 0);
    assert(serial);
    return serial;
}

static uint64_t read_register(ghostos_vm_serial *serial, uint16_t offset) {
    uint64_t value = 0;
    assert(ghostos_vm_serial_read(serial,
        (uint16_t)(ghostos_vm_serial_base(serial) + offset), 1, &value) == 0);
    return value;
}

static bool write_register(ghostos_vm_serial *serial, uint16_t offset, uint8_t value) {
    bool interrupt = false;
    assert(ghostos_vm_serial_write(serial,
        (uint16_t)(ghostos_vm_serial_base(serial) + offset), value, 1, &interrupt) == 0);
    return interrupt;
}

static void host_append(ghostos_vm_serial *serial, const char *bytes) {
    assert(ghostos_vm_serial_append_host_output(serial, (const uint8_t *)bytes, strlen(bytes)));
}

static size_t prepare(ghostos_vm_serial *serial) {
    size_t writable = 0;
    assert(ghostos_vm_serial_prepare_host_output(serial, &writable));
    return writable;
}

static void expect_host_prefix(ghostos_vm_serial *serial, size_t writable, const char *expected) {
    size_t length = 0;
    const uint8_t *bytes = ghostos_vm_serial_host_output(serial, &length);
    assert(writable <= length && writable == strlen(expected));
    if (writable) assert(memcmp(bytes, expected, writable) == 0);
}

static void expect_host(ghostos_vm_serial *serial, const char *expected) {
    size_t length = 0;
    (void)ghostos_vm_serial_host_output(serial, &length);
    expect_host_prefix(serial, length, expected);
}

static void set_banner(ghostos_vm_serial *serial, const char *banner) {
    assert(ghostos_vm_serial_set_banner(serial, (const uint8_t *)banner, strlen(banner)));
}

static void serial_line_control_toggles_dlab(void) {
    ghostos_vm_serial *serial = new_serial();
    (void)write_register(serial, 0, 'A');
    assert(ghostos_vm_serial_tx_count(serial) == 1);
    (void)write_register(serial, 3, 0x80);
    assert(read_register(serial, 3) & 0x80);
    (void)write_register(serial, 0, 1);
    (void)write_register(serial, 1, 0);
    assert(read_register(serial, 0) == 1);
    (void)write_register(serial, 3, 3);
    assert(!(read_register(serial, 3) & 0x80));
    (void)write_register(serial, 0, 'B');
    assert(ghostos_vm_serial_tx_count(serial) == 2);
    ghostos_vm_serial_free(serial);
}

static void serial_rejects_non_byte_accesses(void) {
    ghostos_vm_serial *serial = ghostos_vm_serial_new(0x2f8, 0);
    assert(serial);
    uint64_t value = 0;
    bool interrupt = false;
    assert(ghostos_vm_serial_read(serial, 0x2f8, 2, &value) == 1);
    assert(ghostos_vm_serial_write(serial, 0x2f8, 0x1234, 2, &interrupt) == 1);
    ghostos_vm_serial_free(serial);
}

static void serial_receives_input_and_reports_data_ready(void) {
    ghostos_vm_serial *serial = new_serial();
    (void)ghostos_vm_serial_push_input(serial, (const uint8_t *)"hi", 2);
    assert((read_register(serial, 5) & 1) == 1);
    assert(read_register(serial, 0) == 'h');
    assert(read_register(serial, 0) == 'i');
    assert((read_register(serial, 5) & 1) == 0);
    ghostos_vm_serial_free(serial);
}

static void serial_flushes_output_on_carriage_return(void) {
    ghostos_vm_serial *serial = new_serial();
    (void)write_register(serial, 0, '\r');
    assert(ghostos_vm_serial_tx_count(serial) == 0);
    size_t length = 0;
    const uint8_t *bytes = ghostos_vm_serial_output(serial, &length);
    assert(length == 1 && bytes[0] == '\r');
    ghostos_vm_serial_free(serial);
}

static void authentication_banner_holds_setup_until_prompt(void) {
    ghostos_vm_serial *serial = new_serial();
    set_banner(serial, "Passkey setup and login: http://localhost:1234/?code=test\r\n");
    host_append(serial, "Ready\r\n\x1b]GhostOSEnroll\x07" "Do not type credentials in this terminal.\r\n");
    size_t writable = prepare(serial);
    expect_host_prefix(serial, writable, "Ready\r\n");
    ghostos_vm_serial_consume_host_output(serial, writable);
    assert(ghostos_vm_serial_auth_waiting(serial));
    expect_host(serial, "Do not type credentials in this terminal.\r\n");
    host_append(serial, "Administrator account committed.\r\nGhostOS user shell\r\n$ ");
    writable = prepare(serial);
    assert(!ghostos_vm_serial_auth_waiting(serial));
    expect_host_prefix(serial, writable, "$ ");
    ghostos_vm_serial_free(serial);
}

static void partial_authentication_marker_waits_for_next_flush(void) {
    ghostos_vm_serial *serial = new_serial();
    set_banner(serial, "Passkey URL\r\n");
    host_append(serial, "Ready\r\n\x1b]Ghost");
    size_t writable = prepare(serial);
    expect_host_prefix(serial, writable, "Ready\r\n");
    ghostos_vm_serial_consume_host_output(serial, writable);
    host_append(serial, "OSEnroll\x07");
    assert(prepare(serial) == 0);
    assert(ghostos_vm_serial_auth_waiting(serial));
    expect_host(serial, "");
    ghostos_vm_serial_free(serial);
}

static void authentication_marker_becomes_manual_prompt_without_banner(void) {
    ghostos_vm_serial *serial = new_serial();
    host_append(serial, "\x1b]GhostOSEnroll\x07");
    expect_host_prefix(serial, prepare(serial), "Administrator username: ");
    expect_host(serial, "Administrator username: ");
    assert(!ghostos_vm_serial_auth_waiting(serial));
    ghostos_vm_serial_free(serial);
}

static void login_marker_holds_output_until_prompt(void) {
    ghostos_vm_serial *serial = new_serial();
    set_banner(serial, "Passkey URL\r\n");
    host_append(serial, "\x1b]GhostOSLogin\x07");
    host_append(serial, "Username: micky\r\nLogin accepted.\r\n");
    assert(prepare(serial) == 0);
    assert(ghostos_vm_serial_auth_waiting(serial));
    host_append(serial, "$ ");
    expect_host_prefix(serial, prepare(serial), "$ ");
    assert(!ghostos_vm_serial_auth_waiting(serial));
    ghostos_vm_serial_free(serial);
}

static void extra_authorized_prompt_is_not_reprinted(void) {
    ghostos_vm_serial *serial = new_serial();
    set_banner(serial, "Passkey URL\r\n");
    host_append(serial, "$ ");
    size_t writable = prepare(serial);
    expect_host_prefix(serial, writable, "$ ");
    ghostos_vm_serial_consume_host_output(serial, writable);
    host_append(serial, "\n$ ");
    assert(prepare(serial) == 0);
    assert(!ghostos_vm_serial_auth_waiting(serial));
    ghostos_vm_serial_free(serial);
}

static void leftover_prompt_does_not_end_a_later_login_wait(void) {
    ghostos_vm_serial *serial = new_serial();
    set_banner(serial, "Passkey URL\r\n");
    host_append(serial, "$ ");
    ghostos_vm_serial_consume_host_output(serial, prepare(serial));
    host_append(serial, "\x1b]GhostOSLogin\x07");
    host_append(serial, "$ ");
    assert(prepare(serial) == 0);
    assert(ghostos_vm_serial_auth_waiting(serial));
    host_append(serial, "\n$ ");
    expect_host_prefix(serial, prepare(serial), "$ ");
    assert(!ghostos_vm_serial_auth_waiting(serial));
    ghostos_vm_serial_free(serial);
}

static void authentication_progress_frames_do_not_reprint_after_prompt(void) {
    ghostos_vm_serial *serial = new_serial();
    set_banner(serial, "Passkey URL\r\n");
    host_append(serial, "$ \rGhostOS authentication: |");
    host_append(serial, "\rGhostOS authentication: /");
    host_append(serial, "\rGhostOS authentication: ready\n");
    expect_host_prefix(serial, prepare(serial), "$ ");
    assert(!ghostos_vm_serial_auth_waiting(serial));
    ghostos_vm_serial_free(serial);
}

static void serial_receive_fifo_sets_overrun_and_irq(void) {
    ghostos_vm_serial *serial = new_serial();
    ghostos_vm_apic apic;
    ghostos_vm_apic_init(&apic, 0);
    (void)write_register(serial, 1, 1);
    uint8_t input[GHOSTOS_VM_SERIAL_FIFO_SIZE + 1];
    memset(input, 0xaa, sizeof(input));
    if (ghostos_vm_serial_push_input(serial, input, sizeof(input)))
        ghostos_vm_apic_signal(&apic, 0x24, false);
    assert(ghostos_vm_serial_rx_count(serial) == GHOSTOS_VM_SERIAL_FIFO_SIZE);
    assert(read_register(serial, 5) & 2);
    assert(ghostos_vm_apic_pending(&apic) == 0x24);
    ghostos_vm_serial_reset(serial, 0);
    assert(!ghostos_vm_serial_input_pending(serial));
    ghostos_vm_serial_free(serial);
}

static void lossless_host_input_drains_past_fifo_capacity(void) {
    ghostos_vm_serial *serial = new_serial();
    static const uint8_t input[] = "0123456789abcdefghijklmnop\r";
    bool interrupt = false;
    assert(ghostos_vm_serial_push_input_lossless(serial, input, sizeof(input) - 1, &interrupt));
    for (size_t i = 0; i < sizeof(input) - 1; ++i) assert(read_register(serial, 0) == input[i]);
    assert(!ghostos_vm_serial_input_pending(serial));
    ghostos_vm_serial_free(serial);
}

static void host_console_translates_lf_without_breaking_crlf(void) {
    static const uint8_t input[] = "one\ntwo\r\n";
    uint8_t output[32];
    size_t length = 0;
    bool previous_cr = false;
    assert(ghostos_vm_serial_translate_newlines(input, sizeof(input) - 1, false,
        output, sizeof(output), &length, &previous_cr));
    assert(length == sizeof("one\r\ntwo\r\n") - 1);
    assert(memcmp(output, "one\r\ntwo\r\n", length) == 0);
}

int main(void) {
    serial_line_control_toggles_dlab();
    serial_rejects_non_byte_accesses();
    serial_receives_input_and_reports_data_ready();
    serial_flushes_output_on_carriage_return();
    authentication_banner_holds_setup_until_prompt();
    partial_authentication_marker_waits_for_next_flush();
    authentication_marker_becomes_manual_prompt_without_banner();
    login_marker_holds_output_until_prompt();
    extra_authorized_prompt_is_not_reprinted();
    leftover_prompt_does_not_end_a_later_login_wait();
    authentication_progress_frames_do_not_reprint_after_prompt();
    serial_receive_fifo_sets_overrun_and_irq();
    lossless_host_input_drains_past_fifo_capacity();
    host_console_translates_lf_without_breaking_crlf();
    puts("C VM serial contracts passed");
    return 0;
}
