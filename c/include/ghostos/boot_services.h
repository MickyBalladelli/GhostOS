#ifndef GHOSTOS_BOOT_SERVICES_H
#define GHOSTOS_BOOT_SERVICES_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include "ghostos/status.h"

#define GHOSTOS_BOOT_SERVICE_COUNT 13u
#define GHOSTOS_BOOT_SERVICE_MAX_DEPENDENCIES 5u
#define GHOSTOS_BOOT_FILESYSTEM_PATH_BYTES 64u
#define GHOSTOS_BOOT_ADMIN_USERNAME_BYTES 32u
#define GHOSTOS_BOOT_ADMIN_CREDENTIAL_BYTES 96u
#define GHOSTOS_BOOT_AUTH_DATABASE_BYTES 132u
#define GHOSTOS_BOOT_PASSKEY_COUNTERS_BYTES 4096u
#define GHOSTOS_BOOT_PASSKEY_MAX_KEYS 4u

enum {
    GHOSTOS_BOOT_SERVICE_FILESYSTEM = 0x46534444u,
    GHOSTOS_BOOT_SERVICE_STORAGE = 0x53544f52u,
    GHOSTOS_BOOT_SERVICE_NETWORK = 0x4e455444u,
    GHOSTOS_BOOT_SERVICE_LOGGING = 0x4c4f4744u,
    GHOSTOS_BOOT_SERVICE_AUDIT = 0x41554454u,
    GHOSTOS_BOOT_SERVICE_AUTHENTICATION = 0x41555448u,
    GHOSTOS_BOOT_SERVICE_PACKAGE = 0x504b4744u,
    GHOSTOS_BOOT_SERVICE_SHELL = 0x5348454cu,
    GHOSTOS_BOOT_SERVICE_LOGIN = 0x4c4f4749u,
    GHOSTOS_BOOT_SERVICE_PCI = 0x50434944u,
    GHOSTOS_BOOT_SERVICE_AHCI = 0x41484349u,
    GHOSTOS_BOOT_SERVICE_NVME = 0x4e564d45u,
    GHOSTOS_BOOT_SERVICE_ETHERNET = 0x45544844u
};

typedef enum {
    GHOSTOS_SERVICE_SYSTEM = 1,
    GHOSTOS_SERVICE_STORAGE_DRIVER = 2,
    GHOSTOS_SERVICE_NETWORK_DRIVER = 3
} ghostos_service_kind;

typedef struct {
    uint32_t id;
    uint64_t process_id;
    uint64_t image_id_low;
    uint64_t image_id_high;
    uint64_t capability_profile;
    const char *name;
    ghostos_service_kind kind;
    uint16_t max_restarts;
    uint64_t restart_window_us;
    uint64_t initial_backoff_us;
    uint64_t max_backoff_us;
    uint8_t dependency_count;
    uint32_t dependencies[GHOSTOS_BOOT_SERVICE_MAX_DEPENDENCIES];
} ghostos_boot_service_spec;

typedef struct {
    uint32_t service_id;
    uint64_t process_id;
    size_t startup_order;
    size_t dependency_count;
    uint32_t blocked_on;
    bool has_blocked_on;
    bool ready;
} ghostos_boot_startup_diagnostic;

typedef enum {
    GHOSTOS_FILESYSTEM_READ = 1u,
    GHOSTOS_FILESYSTEM_WRITE = 2u,
    GHOSTOS_FILESYSTEM_DELETE = 4u,
    GHOSTOS_FILESYSTEM_ADMIN = 8u
} ghostos_filesystem_right;

typedef struct {
    uint8_t username_length;
    uint8_t username[32];
    uint8_t key_fingerprint[32];
    uint32_t sign_count;
} ghostos_local_passkey_record;

typedef struct {
    ghostos_status (*lookup)(void *context, const char *path, bool *regular,
                             size_t *size);
    ghostos_status (*read)(void *context, const char *path, void *buffer,
                           size_t capacity, size_t *length);
    ghostos_status (*transaction_begin)(void *context, void **transaction);
    ghostos_status (*transaction_mkdir)(void *context, void *transaction,
                                        const char *path);
    ghostos_status (*transaction_write)(void *context, void *transaction,
                                        const char *path, const void *bytes,
                                        size_t length);
    ghostos_status (*transaction_delete)(void *context, void *transaction,
                                         const char *path);
    ghostos_status (*transaction_commit)(void *context, void *transaction);
    void (*transaction_abort)(void *context, void *transaction);
    ghostos_status (*sync)(void *context);
    ghostos_status (*validate_passkey)(void *context, const uint8_t *key,
                                       size_t key_length);
    ghostos_status (*register_process)(void *context, uint64_t process_id,
                                       uint32_t rights, uint64_t *capability);
    ghostos_status (*unregister_process)(void *context, uint64_t process_id);
    ghostos_status (*dispatch)(void *context, uint64_t process_id,
                               uint32_t operation, uint16_t flags,
                               uint64_t capability, uint64_t offset,
                               uint64_t length, void *buffer,
                               uint64_t values[4]);
    void (*release_owned_filesystem)(void *context);
} ghostos_boot_filesystem_ops;

typedef struct {
    ghostos_boot_filesystem_ops filesystem;
    void *filesystem_context;
    bool owns_filesystem;
    bool provisioning_required;
    bool first_admin_sync_pending;
    bool recovery_sync_pending;
    uint64_t shell_filesystem_capability;
    uint32_t shell_filesystem_rights;
    uint8_t first_admin_username_length;
    uint8_t first_admin_username[GHOSTOS_BOOT_ADMIN_USERNAME_BYTES];
} ghostos_boot_state;

typedef bool (*ghostos_boot_service_launch)(void *context,
                                            const ghostos_boot_service_spec *service,
                                            void *filesystem_context,
                                            uint64_t *process_id);

typedef bool (*ghostos_shell_filesystem_dispatch)(void *context,
                                                 uint64_t process_id,
                                                 uint32_t operation,
                                                 uint16_t flags,
                                                 uint64_t capability,
                                                 uint64_t offset,
                                                 uint64_t length,
                                                 void *buffer,
                                                 uint64_t values[4]);

typedef struct {
    ghostos_shell_filesystem_dispatch dispatch;
    void *context;
    uint64_t shell_process_id;
    uint64_t authority;
} ghostos_shell_filesystem;

typedef bool (*ghostos_boot_service_spawn)(void *context,
                                          const ghostos_boot_service_spec *service,
                                          uint64_t *process_id);

const ghostos_boot_service_spec *ghostos_boot_service_specs(size_t *count);
const ghostos_boot_service_spec *ghostos_boot_service_find(uint32_t service_id);
bool ghostos_boot_service_startup_order(uint32_t *service_ids, size_t capacity,
                                        size_t *count);
bool ghostos_boot_process_for_service(uint32_t service_id, uint64_t *process_id);
bool ghostos_boot_services_start(ghostos_boot_service_spawn spawn, void *context,
                                 ghostos_boot_startup_diagnostic *diagnostics,
                                 size_t diagnostic_capacity);
bool ghostos_boot_state_adopt_filesystem(ghostos_boot_state *state,
                                         const ghostos_boot_filesystem_ops *ops,
                                         void *owned_filesystem_context);
void ghostos_boot_state_release_filesystem(ghostos_boot_state *state);
bool ghostos_boot_kernel_launch(ghostos_boot_state *state,
                               ghostos_boot_service_launch launch, void *context,
                               ghostos_boot_startup_diagnostic *diagnostics,
                               size_t diagnostic_capacity);
uint32_t ghostos_boot_service_filesystem_rights(uint32_t service_id);
bool ghostos_boot_first_admin_required(const void *authorization_database,
                                       size_t database_length);
bool ghostos_boot_first_admin_record_valid(const uint8_t *username,
                                           size_t username_length,
                                           const uint8_t *public_key,
                                           size_t public_key_length);
ghostos_status ghostos_boot_first_admin_set_username(ghostos_boot_state *state,
                                                     const uint8_t *username,
                                                     size_t username_length);
ghostos_status ghostos_boot_first_admin_set_credential(ghostos_boot_state *state,
                                                       uint8_t kind,
                                                       const uint8_t *material,
                                                       size_t material_length);
ghostos_status ghostos_boot_first_admin_commit(ghostos_boot_state *state);
ghostos_status ghostos_boot_first_admin_recovery(ghostos_boot_state *state,
                                                 uint64_t action,
                                                 uint64_t values[4]);
bool ghostos_boot_local_passkey_keys(ghostos_boot_state *state,
                                     const uint8_t *username,
                                     size_t username_length,
                                     uint8_t keys[GHOSTOS_BOOT_PASSKEY_MAX_KEYS]
                                                  [GHOSTOS_BOOT_ADMIN_CREDENTIAL_BYTES],
                                     uint8_t lengths[GHOSTOS_BOOT_PASSKEY_MAX_KEYS],
                                     size_t *key_count);
ghostos_status ghostos_boot_local_passkey_sign_count(ghostos_boot_state *state,
                                                      const uint8_t *username,
                                                      size_t username_length,
                                                      const uint8_t key_fingerprint[32],
                                                      uint32_t *sign_count);
ghostos_status ghostos_boot_local_passkey_record_sign_count(
    ghostos_boot_state *state, const uint8_t *username, size_t username_length,
    const uint8_t key_fingerprint[32], uint32_t sign_count);
bool ghostos_local_passkey_find(const ghostos_local_passkey_record *records,
                                size_t record_count, const uint8_t *username,
                                size_t username_length,
                                const uint8_t key_fingerprint[32],
                                uint32_t *sign_count);
bool ghostos_local_passkey_record_use(ghostos_local_passkey_record *records,
                                     size_t capacity, size_t *record_count,
                                     const uint8_t *username,
                                     size_t username_length,
                                     const uint8_t key_fingerprint[32],
                                     uint32_t sign_count);
bool ghostos_boot_shell_filesystem_dispatch(ghostos_shell_filesystem_dispatch dispatch,
                                            void *context, uint32_t operation,
                                            uint16_t flags, uint64_t capability,
                                            uint64_t offset, uint64_t length,
                                            void *buffer, uint64_t values[4]);
ghostos_status ghostos_boot_shell_filesystem_set_rights(ghostos_boot_state *state,
                                                        uint32_t rights);
ghostos_status ghostos_boot_shell_filesystem_request(ghostos_boot_state *state,
                                                     uint32_t operation,
                                                     uint16_t flags,
                                                     uint64_t capability,
                                                     uint64_t offset,
                                                     uint64_t length,
                                                     void *buffer,
                                                     uint64_t values[4]);
bool ghostos_boot_shell_filesystem_bind(ghostos_shell_filesystem *filesystem,
                                        ghostos_shell_filesystem_dispatch dispatch,
                                        void *context, uint64_t authority);
void ghostos_boot_shell_filesystem_unbind(ghostos_shell_filesystem *filesystem);
bool ghostos_boot_shell_filesystem_call(ghostos_shell_filesystem *filesystem,
                                        uint32_t operation, uint16_t flags,
                                        uint64_t capability, uint64_t offset,
                                        uint64_t length, void *buffer,
                                        uint64_t values[4]);

#endif
