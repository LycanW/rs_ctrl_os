/**
 * Minimal C client: load config, discovery, pub/sub loop.
 * Build (adjust paths to your extracted release or build tree):
 *
 *   gcc -O2 -o minimal minimal.c \
 *     /path/to/librs_ctrl_os.a \
 *     -lzmq -lstdc++ -lpthread -ldl -lm
 *
 * Run: ./minimal /path/to/config.toml
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "rs_ctrl_os.h"

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: %s <config.toml>\n", argv[0]);
        return 1;
    }

    rs_ctrl_os_init_logging();

    RcOsConfig *cfg = rs_ctrl_os_config_open(argv[1]);
    if (!cfg) {
        char err[512];
        rs_ctrl_os_last_error(err, sizeof(err));
        fprintf(stderr, "config_open failed: %s\n", err);
        return 1;
    }

    RcOsTimeSyncHandle *ts = rs_ctrl_os_time_sync_new();
    char *my_id = rs_ctrl_os_config_get_my_id(cfg);
    char *host = rs_ctrl_os_config_get_host(cfg);
    if (!my_id || !host) {
        fprintf(stderr, "missing my_id/host\n");
        if (my_id) rs_ctrl_os_str_free(my_id);
        if (host) rs_ctrl_os_str_free(host);
        rs_ctrl_os_time_sync_destroy(ts);
        rs_ctrl_os_config_destroy(cfg);
        return 1;
    }

    RcOsServiceRegistry *reg = rs_ctrl_os_discovery_start(
        my_id,
        host,
        rs_ctrl_os_config_get_port(cfg),
        rs_ctrl_os_config_get_is_master(cfg),
        ts);
    rs_ctrl_os_str_free(my_id);
    rs_ctrl_os_str_free(host);

    if (!reg) {
        char err[512];
        rs_ctrl_os_last_error(err, sizeof(err));
        fprintf(stderr, "discovery_start failed: %s\n", err);
        rs_ctrl_os_time_sync_destroy(ts);
        rs_ctrl_os_config_destroy(cfg);
        return 1;
    }

    RcOsPubSub *bus = rs_ctrl_os_pubsub_new(cfg, reg);
    if (!bus) {
        char err[512];
        rs_ctrl_os_last_error(err, sizeof(err));
        fprintf(stderr, "pubsub_new failed: %s\n", err);
        rs_ctrl_os_registry_destroy(reg);
        rs_ctrl_os_time_sync_destroy(ts);
        rs_ctrl_os_config_destroy(cfg);
        return 1;
    }

    char *dj = NULL;
    if (rs_ctrl_os_config_get_dynamic_json(cfg, &dj) == RCOS_OK && dj) {
        printf("dynamic json: %s\n", dj);
        rs_ctrl_os_str_free(dj);
    }

    /* Example: publish_raw on first publisher topic if any (requires static_config publishers). */
    (void)rs_ctrl_os_pubsub_publish_raw(bus, "control", "c_hello", (const uint8_t *)"hi", 2);

    for (int i = 0; i < 5; i++) {
        char *st = NULL;
        uint8_t *pl = NULL;
        size_t plen = 0;
        int got = 0;
        rcos_err_t r = rs_ctrl_os_pubsub_try_recv_raw(bus, "local_sub", &st, &pl, &plen, &got);
        if (r != RCOS_OK) {
            char err[256];
            rs_ctrl_os_last_error(err, sizeof(err));
            fprintf(stderr, "recv err %d: %s\n", r, err);
            break;
        }
        if (got && st) {
            printf("recv sub_topic=%s len=%zu\n", st, plen);
            rs_ctrl_os_str_free(st);
            rs_ctrl_os_payload_free(pl, plen);
        }
    }

    rs_ctrl_os_pubsub_destroy(bus);
    rs_ctrl_os_time_sync_destroy(ts);
    rs_ctrl_os_config_destroy(cfg);
    return 0;
}
