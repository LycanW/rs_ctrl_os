/* For usleep() on Linux/glibc with C11 */
#define _DEFAULT_SOURCE

/**
 * rs_ctrl_os C API — 较完整示例（C11）
 *
 * 演示：
 *   - 打开 TOML、读取 static_config（getter）
 *   - 周期性 rs_ctrl_os_config_get_dynamic_toml，得到与文件 [dynamic] 一致的 **TOML 片段**（非 JSON）
 *   - 极简解析 message_prefix、interval_ms（生产请用 libtoml/tomlc99 等）
 *   - discovery + PubSub + publish_raw / try_recv_raw + 时间同步
 *
 * 热更新：编辑同一文件的 [dynamic] 并保存；dynamic_load_enable=true 时 get_dynamic_toml 内容会变。
 */
#include <ctype.h>
#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "rs_ctrl_os.h"

#define DYN_PREFIX_MAX 256
#define PAYLOAD_MAX 512

static const char *skip_ws(const char *p) {
    while (*p == ' ' || *p == '\t' || *p == '\n' || *p == '\r') {
        ++p;
    }
    return p;
}

/** Find line containing `key` at line start (after optional ws), then parse `= "value"` or `= value` */
static int toml_line_string(const char *tbl, const char *key, char *out, size_t cap) {
    const char *p = tbl;
    size_t keylen = strlen(key);
    while (p && *p) {
        const char *line = p;
        const char *nl = strchr(p, '\n');
        size_t linelen = nl ? (size_t)(nl - p) : strlen(p);
        const char *end = p + linelen;
        const char *s = skip_ws(line);
        if ((size_t)(end - s) >= keylen && strncmp(s, key, keylen) == 0) {
            unsigned char c = (unsigned char)s[keylen];
            if (isalnum(c) || c == '_') {
                goto next_line;
            }
            const char *after = s + keylen;
            after = skip_ws(after);
            if (*after != '=') {
                return -1;
            }
            after = skip_ws(after + 1);
            size_t i = 0;
            if (*after == '"') {
                ++after;
                while (*after && *after != '"' && i + 1 < cap) {
                    out[i++] = *after++;
                }
            } else {
                while (after < end && !isspace((unsigned char)*after) && *after != '#' && i + 1 < cap) {
                    out[i++] = *after++;
                }
            }
            out[i] = '\0';
            return 0;
        }
    next_line:
        if (!nl) {
            break;
        }
        p = nl + 1;
    }
    return -1;
}

static int toml_line_u64(const char *tbl, const char *key, uint64_t *out) {
    const char *p = tbl;
    size_t keylen = strlen(key);
    while (p && *p) {
        const char *line = p;
        const char *nl = strchr(p, '\n');
        size_t linelen = nl ? (size_t)(nl - p) : strlen(p);
        const char *end = p + linelen;
        const char *s = skip_ws(line);
        if ((size_t)(end - s) >= keylen && strncmp(s, key, keylen) == 0) {
            unsigned char c = (unsigned char)s[keylen];
            if (isalnum(c) || c == '_') {
                goto next_line_u;
            }
            const char *after = s + keylen;
            after = skip_ws(after);
            if (*after != '=') {
                return -1;
            }
            after = skip_ws(after + 1);
            return sscanf(after, "%" SCNu64, out) == 1 ? 0 : -1;
        }
    next_line_u:
        if (!nl) {
            break;
        }
        p = nl + 1;
    }
    return -1;
}

static void print_last_error(const char *where) {
    char buf[512];
    buf[0] = '\0';
    (void)rs_ctrl_os_last_error(buf, sizeof(buf));
    fprintf(stderr, "[%s] %s\n", where, buf);
}

int main(int argc, char **argv) {
    const char *config_path = (argc >= 2) ? argv[1] : "example_config.toml";
    int max_ticks = (argc >= 3) ? atoi(argv[2]) : 600;

    if (max_ticks <= 0) {
        max_ticks = 600;
    }

    (void)rs_ctrl_os_init_logging();

    RcOsConfig *cfg = rs_ctrl_os_config_open(config_path);
    if (!cfg) {
        print_last_error("config_open");
        return 1;
    }

    char *my_id = rs_ctrl_os_config_get_my_id(cfg);
    char *host = rs_ctrl_os_config_get_host(cfg);
    if (!my_id || !host) {
        fprintf(stderr, "missing my_id/host\n");
        if (my_id) {
            rs_ctrl_os_str_free(my_id);
        }
        if (host) {
            rs_ctrl_os_str_free(host);
        }
        rs_ctrl_os_config_destroy(cfg);
        return 1;
    }

    printf("static: my_id=%s host=%s port=%u master=%d publish_hz=%" PRId64 " subscribe_hz=%" PRId64
           " dynamic_hot_reload=%d\n",
           my_id,
           host,
           (unsigned)rs_ctrl_os_config_get_port(cfg),
           rs_ctrl_os_config_get_is_master(cfg),
           (int64_t)rs_ctrl_os_config_get_publish_hz(cfg),
           (int64_t)rs_ctrl_os_config_get_subscribe_hz(cfg),
           rs_ctrl_os_config_get_dynamic_load_enable(cfg));

    RcOsTimeSyncHandle *ts = rs_ctrl_os_time_sync_new();
    RcOsServiceRegistry *reg =
        rs_ctrl_os_discovery_start(my_id, host, rs_ctrl_os_config_get_port(cfg),
                                   rs_ctrl_os_config_get_is_master(cfg), ts);
    rs_ctrl_os_str_free(my_id);
    rs_ctrl_os_str_free(host);

    if (!reg) {
        print_last_error("discovery_start");
        rs_ctrl_os_time_sync_destroy(ts);
        rs_ctrl_os_config_destroy(cfg);
        return 1;
    }

    RcOsPubSub *bus = rs_ctrl_os_pubsub_new(cfg, reg);
    if (!bus) {
        print_last_error("pubsub_new");
        rs_ctrl_os_time_sync_destroy(ts);
        rs_ctrl_os_config_destroy(cfg);
        return 1;
    }

    char prefix[DYN_PREFIX_MAX] = "hello";
    uint64_t interval_ms = 200;
    char prev_dyn[1024] = "";

    for (int tick = 0; tick < max_ticks; ++tick) {
        char *dyn = NULL;
        if (rs_ctrl_os_config_get_dynamic_toml(cfg, &dyn) != RCOS_OK || !dyn) {
            print_last_error("get_dynamic_toml");
            break;
        }

        if (strcmp(dyn, prev_dyn) != 0) {
            printf("[dynamic] TOML updated:\n%s\n", dyn);
            strncpy(prev_dyn, dyn, sizeof(prev_dyn) - 1);
            prev_dyn[sizeof(prev_dyn) - 1] = '\0';
        }

        {
            char tmp[DYN_PREFIX_MAX];
            uint64_t iv = interval_ms;
            if (toml_line_string(dyn, "message_prefix", tmp, sizeof(tmp)) == 0) {
                if (strcmp(tmp, prefix) != 0) {
                    printf("[dynamic] message_prefix -> %s\n", tmp);
                    strncpy(prefix, tmp, sizeof(prefix) - 1);
                    prefix[sizeof(prefix) - 1] = '\0';
                }
            }
            if (toml_line_u64(dyn, "interval_ms", &iv) == 0 && iv > 0 && iv != interval_ms) {
                printf("[dynamic] interval_ms -> %" PRIu64 "\n", iv);
                interval_ms = iv;
            }
        }

        rs_ctrl_os_str_free(dyn);

        uint64_t ts_ms = rs_ctrl_os_time_sync_now_ms(ts);
        int synced = rs_ctrl_os_time_sync_is_synced(ts);
        char payload[PAYLOAD_MAX];
        int n = snprintf(payload,
                         sizeof(payload),
                         "%s|node|%" PRIu64 "|synced=%d",
                         prefix,
                         ts_ms,
                         synced);
        if (n < 0 || (size_t)n >= sizeof(payload)) {
            fprintf(stderr, "payload truncated\n");
        }

        rcos_err_t pr = rs_ctrl_os_pubsub_publish_raw(
            bus, "control", "telemetry", (const uint8_t *)payload, (size_t)strlen(payload));
        if (pr != RCOS_OK) {
            print_last_error("publish_raw");
        }

        for (int r = 0; r < 3; ++r) {
            char *st = NULL;
            uint8_t *pl = NULL;
            size_t plen = 0;
            int got = 0;
            rcos_err_t rr =
                rs_ctrl_os_pubsub_try_recv_raw(bus, "local_sub", NULL, &st, &pl, &plen, &got);
            if (rr != RCOS_OK) {
                print_last_error("try_recv_raw");
                break;
            }
            if (got && st) {
                printf("[recv] sub_topic=%s payload=%.*s\n", st, (int)plen, (const char *)pl);
                rs_ctrl_os_str_free(st);
                rs_ctrl_os_payload_free(pl, plen);
            }
        }

        usleep((useconds_t)(interval_ms * 1000u));
    }

    rs_ctrl_os_pubsub_destroy(bus);
    rs_ctrl_os_time_sync_destroy(ts);
    rs_ctrl_os_config_destroy(cfg);
    printf("done (%d ticks).\n", max_ticks);
    return 0;
}
