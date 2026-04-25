use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use rs_ctrl_os::{
    init_logging, load_config_typed, start_discovery, PubSubManager, Result, TimeSynchronizer,
};

/// Command received via RPC request from the control node.
#[derive(Serialize, Deserialize, Debug)]
struct ArmCommand {
    command: String,
    args: Vec<String>,
}

/// Status sent back via RPC response.
#[derive(Serialize, Deserialize, Debug)]
struct ArmStatus {
    status: String,
    result: String,
}

/// Process one command and produce a status reply.
fn handle_command(cmd: &ArmCommand) -> ArmStatus {
    match cmd.command.as_str() {
        "move_to" => {
            thread::sleep(Duration::from_millis(100));
            ArmStatus {
                status: "ok".into(),
                result: format!("moved to {}", cmd.args.join(", ")),
            }
        }
        "set_speed" => {
            thread::sleep(Duration::from_millis(50));
            ArmStatus {
                status: "ok".into(),
                result: format!("speed set to {}", cmd.args.first().unwrap_or(&"?".into())),
            }
        }
        "grab" => {
            thread::sleep(Duration::from_millis(200));
            ArmStatus {
                status: "ok".into(),
                result: "grab complete".into(),
            }
        }
        "release" => {
            thread::sleep(Duration::from_millis(80));
            ArmStatus {
                status: "ok".into(),
                result: "released".into(),
            }
        }
        _ => ArmStatus {
            status: "error".into(),
            result: format!("unknown command: {}", cmd.command),
        },
    }
}

#[derive(Clone, Deserialize)]
struct DynamicCfg {}

fn main() -> Result<()> {
    init_logging();

    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "rpc_arm_config.toml".to_string());

    let (static_cfg, _dynamic) = load_config_typed::<DynamicCfg>(&config_path)?;

    let time_sync = Arc::new(TimeSynchronizer::new());

    let registry = start_discovery(
        &static_cfg.my_id,
        &static_cfg.host,
        static_cfg.port,
        static_cfg.is_master,
        Some(time_sync.clone()),
    )?;

    let mut bus = PubSubManager::new(&static_cfg, registry)?;

    // Only accept RPC request messages with sub_topic "rpc/cmd".
    bus.set_sub_topics("arm_cmd", &["rpc/cmd"])?;

    println!("[arm] Listening for RPC requests... (Ctrl+C to stop)");

    // Receive up to 10 requests then exit (demonstration limit).
    let mut count = 0;
    while count < 10 {
        // try_recv_request returns only messages with a valid RPC request envelope.
        if let Some((sender, rid, topic, payload)) = bus.try_recv_request("arm_cmd")? {
            if let Ok(cmd) = bincode::deserialize::<ArmCommand>(&payload) {
                println!(
                    "[arm]  RPC request #{} rid={} from={} topic={} cmd={:?}",
                    count, rid, sender, topic, cmd
                );

                let status = handle_command(&cmd);
                let response = bincode::serialize(&status)?;

                // Send response back to the control node.
                bus.publish_response("arm_resp", "rpc/status", rid, &response)?;
                println!("[arm]  -> reply: {:?}", status);
                count += 1;
            } else {
                println!("[arm]  RPC request rid={} from={} <deserialize failed>", rid, sender);
            }
        } else {
            thread::sleep(Duration::from_millis(10));
        }
    }

    println!("[arm] Processed {} requests, shutting down.", count);
    Ok(())
}
