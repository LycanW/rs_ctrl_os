/**
 * Minimal C++23 client using rs_ctrl_os C API via CMake.
 *
 * Configure (from c_examples/, default RCOS_ROOT is parent repo):
 *   cmake -S . -B build -DRCOS_ROOT=.. -DRCOS_LIB=../target/release/librs_ctrl_os.a
 *   cmake --build build
 *
 * Run:
 *   ./build/rcos_minimal /path/to/config.toml
 */
#include <cstdint>
#include <format>
#include <iostream>
#include <string>
#include <string_view>

#include "rs_ctrl_os.h"

namespace {

void print_last_error(std::string_view label) {
    char buf[512]{};
    (void)rs_ctrl_os_last_error(buf, sizeof(buf));
    std::cerr << std::format("{}: {}\n", label, buf);
}

}  // namespace

int main(int argc, char **argv) {
    if (argc < 2) {
        std::cerr << std::format("usage: {} <config.toml>\n", argv[0]);
        return 1;
    }

    rs_ctrl_os_init_logging();

    RcOsConfig *cfg = rs_ctrl_os_config_open(argv[1]);
    if (!cfg) {
        print_last_error("config_open");
        return 1;
    }

    RcOsTimeSyncHandle *ts = rs_ctrl_os_time_sync_new();
    char *my_id = rs_ctrl_os_config_get_my_id(cfg);
    char *host = rs_ctrl_os_config_get_host(cfg);
    if (!my_id || !host) {
        std::cerr << "missing my_id/host\n";
        if (my_id) {
            rs_ctrl_os_str_free(my_id);
        }
        if (host) {
            rs_ctrl_os_str_free(host);
        }
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

    char *dj = nullptr;
    if (rs_ctrl_os_config_get_dynamic_json(cfg, &dj) == RCOS_OK && dj) {
        std::cout << std::format("dynamic json: {}\n", static_cast<const char *>(dj));
        rs_ctrl_os_str_free(dj);
    }

    constexpr std::string_view payload{"hi"};
    (void)rs_ctrl_os_pubsub_publish_raw(
        bus,
        "control",
        "c_hello",
        reinterpret_cast<const std::uint8_t *>(payload.data()),
        payload.size());

    for (int i = 0; i < 5; ++i) {
        char *st = nullptr;
        std::uint8_t *pl = nullptr;
        std::size_t plen = 0;
        int got = 0;
        rcos_err_t r =
            rs_ctrl_os_pubsub_try_recv_raw(bus, "local_sub", &st, &pl, &plen, &got);
        if (r != RCOS_OK) {
            print_last_error("try_recv_raw");
            break;
        }
        if (got != 0 && st != nullptr) {
            std::cout << std::format(
                "recv sub_topic={} len={}\n", static_cast<const char *>(st), plen);
            rs_ctrl_os_str_free(st);
            rs_ctrl_os_payload_free(pl, plen);
        }
    }

    rs_ctrl_os_pubsub_destroy(bus);
    rs_ctrl_os_time_sync_destroy(ts);
    rs_ctrl_os_config_destroy(cfg);
    return 0;
}
