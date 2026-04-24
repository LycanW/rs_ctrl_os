use rs_ctrl_os::config::load_config_rcos;
use std::fs;
use std::io::Write;

static CNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn write_tmp(content: &str) -> std::path::PathBuf {
    let n = CNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir();
    let path = dir.join(format!("rcos_test_{}_{}.toml", std::process::id(), n));
    let mut f = fs::File::create(&path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
    path
}

#[test]
fn load_valid_config() {
    let toml = r#"
[static_config]
my_id = "node1"
host = "127.0.0.1"
port = 5555
publish_hz = 100
subscribe_hz = 50

[dynamic]
key = "value"
"#;
    let path = write_tmp(toml);
    let (cfg, dyn_val) = load_config_rcos(&path).unwrap();
    assert_eq!(cfg.my_id, "node1");
    assert_eq!(cfg.port, 5555);
    assert!(!cfg.is_master);
    assert!(cfg.dynamic_load_enable); // default true
    assert_eq!(dyn_val.get("key").unwrap().as_str().unwrap(), "value");
    let _ = fs::remove_file(&path);
}

#[test]
fn load_config_defaults() {
    let toml = r#"
[static_config]
my_id = "n"
host = "0.0.0.0"
port = 1
publish_hz = 0
subscribe_hz = 0
"#;
    let path = write_tmp(toml);
    let (cfg, _) = load_config_rcos(&path).unwrap();
    assert!(!cfg.is_master);
    assert!(cfg.subscribers.is_empty());
    assert!(cfg.publishers.is_empty());
    assert!(cfg.static_nodes.is_empty());
    let _ = fs::remove_file(&path);
}

#[test]
fn missing_static_config_is_error() {
    let toml = r#"
[dynamic]
key = "val"
"#;
    let path = write_tmp(toml);
    let err = load_config_rcos(&path).unwrap_err();
    assert!(err.to_string().contains("static_config"));
    let _ = fs::remove_file(&path);
}

#[test]
fn missing_required_fields_is_error() {
    let toml = r#"
[static_config]
my_id = "n"
"#;
    let path = write_tmp(toml);
    assert!(load_config_rcos(&path).is_err());
    let _ = fs::remove_file(&path);
}

#[test]
fn dynamic_load_enable_false() {
    let toml = r#"
[static_config]
my_id = "n"
host = "0.0.0.0"
port = 1
publish_hz = 0
subscribe_hz = 0
dynamic_load_enable = false
"#;
    let path = write_tmp(toml);
    let (cfg, _) = load_config_rcos(&path).unwrap();
    assert!(!cfg.dynamic_load_enable);
    let _ = fs::remove_file(&path);
}

#[test]
fn missing_dynamic_section_is_empty() {
    let toml = r#"
[static_config]
my_id = "n"
host = "0.0.0.0"
port = 1
publish_hz = 0
subscribe_hz = 0
"#;
    let path = write_tmp(toml);
    let (_, dyn_val) = load_config_rcos(&path).unwrap();
    assert!(dyn_val.as_table().unwrap().is_empty());
    let _ = fs::remove_file(&path);
}
