/* For usleep() on Linux/glibc with C11 */
#define _DEFAULT_SOURCE

/**
 * rs_ctrl_os C API — 较完整示例（C11）
 *
 * 演示：
 *   - 打开 TOML、读取 static_config（通过 getter）
 *   - 周期性 rs_ctrl_os_config_get_dynamic_json，解析 [dynamic]（本示例仅解析 message_prefix、interval_ms）
 *   - 时间同步 + discovery + PubSub：按 dynamic 中的间隔发 publish_raw，前缀随热更新变化
 *   - try_recv_raw 收包并释放
 *
 * 热更新自测：保持本进程运行，另开终端编辑同一 TOML 的 [dynamic] 并保存；
 * 若 dynamic_load_enable=true，日志里会看到前缀/间隔随 JSON 变化。
 *
 * 依赖：仅 C 标准库 + rs_ctrl_os 头文件/静态库；JSON 为极简手写解析，仅适配方括号 TOML 转 JSON 的常见形态。
 * 生产环境建议换 jansson、cJSON 等库解析 get_dynamic_json 的字符串。
 */
#include <ctype.h>
#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h> /* usleep */

#include "rs_ctrl_os.h"

#define DYN_PREFIX_MAX 256
#define PAYLOAD_MAX 512

static const char *skip_ws(const char *p) {
    while (*p == ' ' || *p == '\t' || *p == '\n' || *p == '\r') {
        ++p;
    }
    return p;
}

/**
 * 从 JSON 文本中取 "key":"string_value" 的字符串（不做转义处理，够用示例配置）。
 * 返回 0 成功，-1 失败。
 */
static int json_extract_string(const char *json, const char *key, char *out, size_t cap) {
    char needle[80];
    if (snprintf(needle, sizeof(needle), "\"%s\"", key) >= (int)sizeof(needle)) {
        return -1;
    }
    const char *p = strstr(json, needle);
    if (!p) {
        return -1;
    }
    p = strchr(p, ':');
    if (!p) {
        return -1;
    }
    p = skip_ws(p + 1);
    if (*p != '"') {
        return -1;
    }
    ++p;
    size_t i = 0;
    while (*p && *p != '"' && i + 1 < cap) {
        out[i++] = *p++;
    }
    out[i] = '\0';
    return 0;
}

/** 从 JSON 中取 "key":123 的无符号整数 */
static int json_extract_u64(const char *json, const char *key, uint64_t *out) {
    char needle[80];
    if (snprintf(needle, sizeof(needle), "\"%s\"", key) >= (int)sizeof(needle)) {
        return -1;
    }
    const char *p = strstr(json, needle);
    if (!p) {
        return -1;
    }
    p = strchr(p, ':');
    if (!p) {
        return -1;
    }
    p = skip_ws(p + 1);
    return sscanf(p, "%" SCNu64, out) == 1 ? 0 : -1;
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
        rs_ctrl_os_registry_destroy(reg);
        rs_ctrl_os_time_sync_destroy(ts);
        rs_ctrl_os_config_destroy(cfg);
        return 1;
    }

    char prefix[DYN_PREFIX_MAX] = "hello";
    uint64_t interval_ms = 200;
    char prev_json[1024] = "";

    for (int tick = 0; tick < max_ticks; ++tick) {
        char *json = NULL;
        if (rs_ctrl_os_config_get_dynamic_json(cfg, &json) != RCOS_OK || !json) {
            print_last_error("get_dynamic_json");
            break;
        }

        if (strcmp(json, prev_json) != 0) {
            printf("[dynamic] JSON updated: %s\n", json);
            strncpy(prev_json, json, sizeof(prev_json) - 1);
            prev_json[sizeof(prev_json) - 1] = '\0';
        }

        {
            char tmp[DYN_PREFIX_MAX];
            uint64_t iv = interval_ms;
            if (json_extract_string(json, "message_prefix", tmp, sizeof(tmp)) == 0) {
                if (strcmp(tmp, prefix) != 0) {
                    printf("[dynamic] message_prefix -> %s\n", tmp);
                    strncpy(prefix, tmp, sizeof(prefix) - 1);
                    prefix[sizeof(prefix) - 1] = '\0';
                }
            }
            if (json_extract_u64(json, "interval_ms", &iv) == 0 && iv > 0 && iv != interval_ms) {
                printf("[dynamic] interval_ms -> %" PRIu64 "\n", iv);
                interval_ms = iv;
            }
        }

        rs_ctrl_os_str_free(json);

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
                rs_ctrl_os_pubsub_try_recv_raw(bus, "local_sub", &st, &pl, &plen, &got);
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
