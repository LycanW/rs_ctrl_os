use rs_ctrl_os::discovery::{Heartbeat, ServiceRegistry};

#[test]
fn shutdown_signals_running_flag() {
    let registry = ServiceRegistry::new();
    let hb = Heartbeat {
        node_id: "node".to_string(),
        host: "127.0.0.1".to_string(),
        port: 5555,
        timestamp: 0,
        clock_time_ms: 0,
        is_master: false,
    };
    registry.register(&hb);
    assert!(registry.get_address("node").is_some());

    registry.shutdown();
    // Shutdown doesn't affect existing data, just signals background threads.
    // After shutdown, the registry is still usable for reads.
    assert!(registry.get_address("node").is_some());
}
