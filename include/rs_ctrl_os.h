/**
 * rs_ctrl_os — C ABI (Linux glibc, x86_64 / aarch64).
 * Link with `librs_ctrl_os.a`, system libzmq (`-lzmq`), bundled libzmq C++ objects require `-lstdc++`,
 * and typically `-lpthread -ldl -lm`.
 *
 * UTF-8 NUL-terminated strings for all `const char *` path and name parameters.
 * Strings returned by getters / try_recv must be freed with rs_ctrl_os_str_free or rs_ctrl_os_payload_free.
 */
#ifndef RS_CTRL_OS_H
#define RS_CTRL_OS_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef int rcos_err_t;

#define RCOS_OK 0
#define RCOS_ERR_INVALID 1
#define RCOS_ERR_UTF8 2
#define RCOS_ERR_CONFIG 3
#define RCOS_ERR_COMMS 4
#define RCOS_ERR_DISCOVERY 5
#define RCOS_ERR_SERIALIZATION 6
#define RCOS_ERR_NODE_NOT_FOUND 7
#define RCOS_ERR_IO 8
#define RCOS_ERR_ZMQ 9
#define RCOS_ERR_BINCODE 10
#define RCOS_ERR_TRUNC 11
#define RCOS_ERR_INTERNAL 99

typedef struct RcOsTimeSyncHandle RcOsTimeSyncHandle;
typedef struct RcOsConfig RcOsConfig;
typedef struct RcOsServiceRegistry RcOsServiceRegistry;
typedef struct RcOsPubSub RcOsPubSub;

/** Initialize tracing (call once). */
rcos_err_t rs_ctrl_os_init_logging(void);

/**
 * Copy last error text into `buf` (NUL-terminated if buf_len > 0).
 * Returns length written excluding NUL, or RCOS_ERR_TRUNC / RCOS_ERR_INVALID.
 */
rcos_err_t rs_ctrl_os_last_error(char *buf, size_t buf_len);

/** Free strings returned by rs_ctrl_os_config_get_* or rs_ctrl_os_pubsub_try_recv_raw (sub_topic_out). */
void rs_ctrl_os_str_free(char *p);

RcOsTimeSyncHandle *rs_ctrl_os_time_sync_new(void);
void rs_ctrl_os_time_sync_destroy(RcOsTimeSyncHandle *p);
uint64_t rs_ctrl_os_time_sync_now_ms(const RcOsTimeSyncHandle *p);
/** 1 if synced, 0 otherwise. */
int rs_ctrl_os_time_sync_is_synced(const RcOsTimeSyncHandle *p);

/** Open TOML config (full file with [static_config] and [dynamic]). NULL on failure. */
RcOsConfig *rs_ctrl_os_config_open(const char *path_utf8);
void rs_ctrl_os_config_destroy(RcOsConfig *p);

/**
 * Returns a snapshot of the current `[dynamic]` table as TOML text (key/value lines only).
 * File watch + reload when dynamic_load_enable=true are done inside rs_ctrl_os; this call only
 * reads the in-memory value. Caller frees with rs_ctrl_os_str_free. Use a TOML parser in app
 * code to read fields; the library does not interpret [dynamic] schema.
 */
rcos_err_t rs_ctrl_os_config_get_dynamic_toml(const RcOsConfig *cfg, char **out_toml);

char *rs_ctrl_os_config_get_my_id(const RcOsConfig *cfg);
char *rs_ctrl_os_config_get_host(const RcOsConfig *cfg);
uint16_t rs_ctrl_os_config_get_port(const RcOsConfig *cfg);
int rs_ctrl_os_config_get_is_master(const RcOsConfig *cfg);
int64_t rs_ctrl_os_config_get_publish_hz(const RcOsConfig *cfg);
int64_t rs_ctrl_os_config_get_subscribe_hz(const RcOsConfig *cfg);
int rs_ctrl_os_config_get_dynamic_load_enable(const RcOsConfig *cfg);

/**
 * Start UDP discovery. Pass NULL for time_sync if unused.
 * Returns registry handle, or NULL on failure.
 */
RcOsServiceRegistry *rs_ctrl_os_discovery_start(
    const char *my_id,
    const char *my_host,
    uint16_t my_port,
    int is_master,
    const RcOsTimeSyncHandle *time_sync);

/** Only if discovery failed or registry was never passed to pubsub_new. */
void rs_ctrl_os_registry_destroy(RcOsServiceRegistry *p);

/**
 * Create pub/sub manager. CONSUMES `registry` (do not destroy after success).
 * NULL on failure (registry is lost; reopen config and discovery if needed).
 */
RcOsPubSub *rs_ctrl_os_pubsub_new(const RcOsConfig *cfg, RcOsServiceRegistry *registry);
void rs_ctrl_os_pubsub_destroy(RcOsPubSub *p);

void rs_ctrl_os_pubsub_set_publish_hz(RcOsPubSub *bus, int64_t hz);
void rs_ctrl_os_pubsub_set_subscribe_hz(RcOsPubSub *bus, int64_t hz);

rcos_err_t rs_ctrl_os_pubsub_publish_raw(
    RcOsPubSub *bus,
    const char *topic_key,
    const char *sub_topic,
    const uint8_t *payload,
    size_t payload_len);

/**
 * Non-blocking receive. If *got_message_out == 1, *sub_topic_out and *payload_out are allocated;
 * free with rs_ctrl_os_str_free and rs_ctrl_os_payload_free(..., *payload_len_out).
 */
rcos_err_t rs_ctrl_os_pubsub_try_recv_raw(
    RcOsPubSub *bus,
    const char *local_name,
    char **sub_topic_out,
    uint8_t **payload_out,
    size_t *payload_len_out,
    int *got_message_out);

void rs_ctrl_os_payload_free(uint8_t *p, size_t len);

/** topics may be NULL with topic_count 0 to clear filter. */
rcos_err_t rs_ctrl_os_pubsub_set_sub_topics(
    RcOsPubSub *bus,
    const char *local_name,
    const char *const *topics,
    size_t topic_count);

#ifdef __cplusplus
}
#endif

#endif /* RS_CTRL_OS_H */
