//! Smoke test for C ABI symbols (Rust calls them via `ffi` module).
use std::ffi::CString;

#[test]
fn ffi_config_roundtrip_example_toml() {
    let path = CString::new("example_config.toml").expect("path");
    unsafe {
        let cfg = rs_ctrl_os::ffi::rs_ctrl_os_config_open(path.as_ptr());
        assert!(!cfg.is_null(), "config_open failed");

        let port = rs_ctrl_os::ffi::rs_ctrl_os_config_get_port(cfg);
        assert_eq!(port, 5555);

        let mut json_out: *mut std::ffi::c_char = std::ptr::null_mut();
        let r = rs_ctrl_os::ffi::rs_ctrl_os_config_get_dynamic_json(cfg, &mut json_out);
        assert_eq!(r, rs_ctrl_os::ffi::RCOS_OK);
        assert!(!json_out.is_null());
        rs_ctrl_os::ffi::rs_ctrl_os_str_free(json_out);

        rs_ctrl_os::ffi::rs_ctrl_os_config_destroy(cfg);
    }
}
