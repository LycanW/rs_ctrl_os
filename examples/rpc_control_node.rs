use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use rs_ctrl_os::{
    init_logging, load_config_typed, start_discovery, PubSubManager, Result, TimeSynchronizer,
};

/// Command sent via RPC request to the arm node.
#[derive(Serialize, Deserialize, Debug)]
struct ArmCommand {
    command: String,
    args: Vec<String>,
}

/// Status received via RPC response from the arm node.
#[derive(Serialize, Deserialize, Debug)]
struct ArmStatus {
    status: String, // "ok", "error", "busy"
    result: String,
}

#[derive(Clone, Deserialize)]
struct DynamicCfg {}

fn main() -> Result<()> {
    init_logging();

    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "rpc_control_config.toml".to_string());

    let (static_cfg, _dynamic) = load_config_typed::<DynamicCfg>(&config_path)?;

    let time_sync = Arc::new(TimeSynchronizer::new());

    // Participates in discovery to find the arm node.
    let registry = start_discovery(
        &static_cfg.my_id,
        &static_cfg.host,
        static_cfg.port,
        static_cfg.is_master,
        Some(time_sync.clone()),
    )?;

    let mut bus = PubSubManager::new(&static_cfg, registry)?;

    // Demo: send 5 distinct RPC commands, each with its own request_id.
    let commands = vec![
        ArmCommand {
            command: "move_to".into(),
            args: vec!["x:100".into(), "y:200".into()],
        },
        ArmCommand {
            command: "set_speed".into(),
            args: vec!["50".into()],
        },
        ArmCommand {
            command: "grab".into(),
            args: vec!["width:30".into()],
        },
        ArmCommand {
            command: "move_to".into(),
            args: vec!["x:300".into(), "y:400".into()],
        },
        ArmCommand {
            command: "release".into(),
            args: vec![],
        },
    ];

    let mut pending: Vec<u64> = Vec::new();

    for (i, cmd) in commands.into_iter().enumerate() {
        let rid = i as u64;
        let payload = bincode::serialize(&cmd)?;

        println!("[control] Sending RPC request rid={rid} cmd={:?}", cmd);
        bus.publish_request("cmd", "rpc/cmd", rid, &payload)?;
        pending.push(rid);

        thread::sleep(Duration::from_millis(200)); // stagger sends
    }

    // Poll for responses with a timeout.
    println!(
        "[control] Waiting for {} response(s) from arm...",
        pending.len()
    );

    let mut matched = 0u64;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);

    // Filter: only accept RPC responses on this local_name.
    // The sub_topic "rpc/status" is used by the arm node for replies.
    bus.set_sub_topics("ctrl_resp", &["rpc/status"])?;

    while matched < pending.len() as u64 && std::time::Instant::now() < deadline {
        // try_recv_response only returns messages with a valid RPC response envelope.
        if let Some((sender, rid, topic, payload)) = bus.try_recv_response("ctrl_resp")? {
            if let Ok(status) = bincode::deserialize::<ArmStatus>(&payload) {
                println!(
                    "[control] RPC response rid={rid} from={sender} topic={topic} status={:?}",
                    status
                );
                matched += 1;
            } else {
                println!(
                    "[control] RPC response rid={rid} from={sender} <deserialize failed>"
                );
            }
        } else {
            // No message yet — brief sleep to avoid busy spin.
            thread::sleep(Duration::from_millis(10));
        }
    }

    println!(
        "[control] Done. {}/{} RPC responses received.",
        matched,
        pending.len()
    );
    Ok(())
}
