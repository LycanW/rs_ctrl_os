/**
 * C API 收发订阅示例（两进程演示）
 *
 * 终端 1：./rcos_pubsub_chat pub pubsub_chat_pub.toml
 * 终端 2：./rcos_pubsub_chat sub pubsub_chat_sub.toml
 *
 * 发布端用 rs_ctrl_os_pubsub_publish_raw(bus, "control", "chat", payload)。
 * 订阅端用 rs_ctrl_os_pubsub_set_sub_topics 只接收 sub_topic == "chat"，
 * 再用 rs_ctrl_os_pubsub_try_recv_raw 非阻塞收包。
 */
#define _DEFAULT_SOURCE

#include <inttypes.h>
#include <signal.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "rs_ctrl_os.h"

static volatile sig_atomic_t g_stop;

static void on_sigint(int signo) {
    (void)signo;
    g_stop = 1;
}

static void print_last_error(const char *where) {
    char buf[512];
    buf[0] = '\0';
    (void)rs_ctrl_os_last_error(buf, sizeof(buf));
    fprintf(stderr, "[%s] %s\n", where, buf);
}

typedef struct {
    RcOsConfig *cfg;
    RcOsTimeSyncHandle *ts;
    RcOsPubSub *bus;
} Session;

static void session_fini(Session *s) {
    if (s->bus) {
        rs_ctrl_os_pubsub_destroy(s->bus);
    }
    if (s->ts) {
        rs_ctrl_os_time_sync_destroy(s->ts);
    }
    if (s->cfg) {
        rs_ctrl_os_config_destroy(s->cfg);
    }
}

/** 打开配置、discovery、pubsub；失败时 *s 部分有效，调用方再 session_fini */
static int session_init(Session *s, const char *config_path) {
    memset(s, 0, sizeof(*s));

    s->cfg = rs_ctrl_os_config_open(config_path);
    if (!s->cfg) {
        print_last_error("config_open");
        return -1;
    }

    char *my_id = rs_ctrl_os_config_get_my_id(s->cfg);
    char *host = rs_ctrl_os_config_get_host(s->cfg);
    if (!my_id || !host) {
        fprintf(stderr, "missing my_id/host\n");
        if (my_id) {
            rs_ctrl_os_str_free(my_id);
        }
        if (host) {
            rs_ctrl_os_str_free(host);
        }
        session_fini(s);
        return -1;
    }

    s->ts = rs_ctrl_os_time_sync_new();
    RcOsServiceRegistry *reg = rs_ctrl_os_discovery_start(
        my_id, host, rs_ctrl_os_config_get_port(s->cfg), rs_ctrl_os_config_get_is_master(s->cfg), s->ts);
    rs_ctrl_os_str_free(my_id);
    rs_ctrl_os_str_free(host);

    if (!reg) {
        print_last_error("discovery_start");
        session_fini(s);
        return -1;
    }

    s->bus = rs_ctrl_os_pubsub_new(s->cfg, reg);
    if (!s->bus) {
        print_last_error("pubsub_new");
        /* pubsub_new always takes registry; on NULL return the pointer is dangling */
        session_fini(s);
        return -1;
    }
    return 0;
}

static int run_pub(Session *s, unsigned interval_ms) {
    printf("[pub] publishing on topic_key=control sub_topic=chat every %u ms (Ctrl+C to stop)\n",
           interval_ms);
    uint64_t seq = 0;
    while (!g_stop) {
        char line[512];
        int n = snprintf(line, sizeof(line), "seq=%" PRIu64 " t=%" PRIu64, seq++,
                         (uint64_t)rs_ctrl_os_time_sync_now_ms(s->ts));
        if (n < 0 || (size_t)n >= sizeof(line)) {
            fprintf(stderr, "payload truncated\n");
            return 1;
        }
        rcos_err_t e = rs_ctrl_os_pubsub_publish_raw(
            s->bus, "control", "chat", (const uint8_t *)line, (size_t)strlen(line));
        if (e != RCOS_OK) {
            print_last_error("publish_raw");
        } else {
            printf("[pub] sent: %s\n", line);
        }
        usleep((useconds_t)(interval_ms * 1000u));
    }
    return 0;
}

static int run_sub(Session *s) {
    const char *local_name = "recv0";
    const char *only[] = {"chat"};
    rcos_err_t fe =
        rs_ctrl_os_pubsub_set_sub_topics(s->bus, local_name, only, sizeof(only) / sizeof(only[0]));
    if (fe != RCOS_OK) {
        print_last_error("set_sub_topics");
        return 1;
    }
    printf("[sub] recv0: filter sub_topic in { chat }; Ctrl+C to stop\n");

    while (!g_stop) {
        char *st = NULL;
        uint8_t *pl = NULL;
        size_t plen = 0;
        int got = 0;
        rcos_err_t rr = rs_ctrl_os_pubsub_try_recv_raw(s->bus, local_name, &st, &pl, &plen, &got);
        if (rr != RCOS_OK) {
            print_last_error("try_recv_raw");
            return 1;
        }
        if (got && st) {
            printf("[sub] sub_topic=%s payload=%.*s\n", st, (int)plen, pl ? (const char *)pl : "");
            rs_ctrl_os_str_free(st);
            rs_ctrl_os_payload_free(pl, plen);
        }
        usleep(5000);
    }
    return 0;
}

static void usage(const char *argv0) {
    fprintf(stderr,
            "Usage:\n"
            "  %s pub <config.toml> [interval_ms]\n"
            "  %s sub <config.toml>\n"
            "Example (from build dir):\n"
            "  %s pub ../pubsub_chat_pub.toml 500\n"
            "  %s sub ../pubsub_chat_sub.toml\n",
            argv0, argv0, argv0, argv0);
}

int main(int argc, char **argv) {
    if (argc < 3) {
        usage(argv[0]);
        return 1;
    }

    const char *role = argv[1];
    const char *cfgpath = argv[2];
    unsigned interval_ms = 500;
    if (argc >= 4 && strcmp(role, "pub") == 0) {
        interval_ms = (unsigned)strtoul(argv[3], NULL, 10);
        if (interval_ms == 0) {
            interval_ms = 500;
        }
    }

    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = on_sigint;
    sigaction(SIGINT, &sa, NULL);
    sigaction(SIGTERM, &sa, NULL);

    (void)rs_ctrl_os_init_logging();

    Session s;
    if (session_init(&s, cfgpath) != 0) {
        return 1;
    }

    int rc = 0;
    if (strcmp(role, "pub") == 0) {
        rc = run_pub(&s, interval_ms);
    } else if (strcmp(role, "sub") == 0) {
        rc = run_sub(&s);
    } else {
        usage(argv[0]);
        rc = 1;
    }

    session_fini(&s);
    return rc;
}
