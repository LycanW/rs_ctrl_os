//! C ABI for embedding `rs_ctrl_os` from C/C++.
//!
//! All string paths and topic names must be UTF-8 with a trailing NUL.
//! Functions that return `char*` allocate with Rust; free with [`rs_ctrl_os_str_free`].
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use crate::comms::PubSubManager;
use crate::config::ConfigManager;
use crate::discovery::{start_discovery, ServiceRegistry};
use crate::error::RsCtrlError;
use crate::time_sync::TimeSynchronizer;
use libc::{c_char, c_int, size_t};
use std::ffi::{CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::ptr;
use std::sync::Arc;
use std::sync::Mutex;

/// Success.
pub const RCOS_OK: c_int = 0;
/// Null pointer or invalid argument.
pub const RCOS_ERR_INVALID: c_int = 1;
/// UTF-8 decode error.
pub const RCOS_ERR_UTF8: c_int = 2;
pub const RCOS_ERR_CONFIG: c_int = 3;
pub const RCOS_ERR_COMMS: c_int = 4;
pub const RCOS_ERR_DISCOVERY: c_int = 5;
pub const RCOS_ERR_SERIALIZATION: c_int = 6;
pub const RCOS_ERR_NODE_NOT_FOUND: c_int = 7;
pub const RCOS_ERR_IO: c_int = 8;
pub const RCOS_ERR_ZMQ: c_int = 9;
pub const RCOS_ERR_BINCODE: c_int = 10;
/// Buffer too small for output.
pub const RCOS_ERR_TRUNC: c_int = 11;
/// Panic or internal error.
pub const RCOS_ERR_INTERNAL: c_int = 99;

static LAST_ERROR: Mutex<String> = Mutex::new(String::new());

fn set_last_error(msg: impl Into<String>) {
    if let Ok(mut g) = LAST_ERROR.lock() {
        *g = msg.into();
    }
}

fn clear_last_error() {
    if let Ok(mut g) = LAST_ERROR.lock() {
        g.clear();
    }
}

fn map_err(e: RsCtrlError) -> c_int {
    let code = error_code(&e);
    set_last_error(e.to_string());
    code
}

fn error_code(e: &RsCtrlError) -> c_int {
    match e {
        RsCtrlError::Config(_) => RCOS_ERR_CONFIG,
        RsCtrlError::Comms(_) => RCOS_ERR_COMMS,
        RsCtrlError::Serialization(_) => RCOS_ERR_SERIALIZATION,
        RsCtrlError::Discovery(_) => RCOS_ERR_DISCOVERY,
        RsCtrlError::NodeNotFound(_) => RCOS_ERR_NODE_NOT_FOUND,
        RsCtrlError::Io(_) => RCOS_ERR_IO,
        RsCtrlError::Zmq(_) => RCOS_ERR_ZMQ,
        RsCtrlError::Bincode(_) => RCOS_ERR_BINCODE,
    }
}

fn ffi_guard<F, T>(f: F) -> c_int
where
    F: FnOnce() -> Result<T, RsCtrlError>,
{
    clear_last_error();
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(_)) => RCOS_OK,
        Ok(Err(e)) => map_err(e),
        Err(_) => {
            set_last_error("panic in rs_ctrl_os FFI boundary");
            RCOS_ERR_INTERNAL
        }
    }
}

fn ffi_guard_void<F>(f: F) -> c_int
where
    F: FnOnce() -> Result<(), RsCtrlError>,
{
    ffi_guard(|| f().map(|_| ()))
}

/// Wraps `Arc<TimeSynchronizer>` for sharing with discovery.
#[repr(C)]
pub struct RcOsTimeSyncHandle {
    inner: Arc<TimeSynchronizer>,
}

/// Opaque config manager with `toml::Value` dynamic section.
pub type RcOsConfig = ConfigManager<toml::Value>;

// --- init ---

/// Initialize tracing logging (safe to call once at process start).
#[no_mangle]
pub extern "C" fn rs_ctrl_os_init_logging() -> c_int {
    clear_last_error();
    match catch_unwind(AssertUnwindSafe(|| {
        crate::init_logging();
    })) {
        Ok(()) => RCOS_OK,
        Err(_) => {
            set_last_error("panic in rs_ctrl_os_init_logging");
            RCOS_ERR_INTERNAL
        }
    }
}

/// Copy last error message into `buf` (NUL-terminated if space allows). Returns bytes written excluding NUL, or -1 if truncated (message still partially written with NUL if `buf_len` > 0).
#[no_mangle]
pub extern "C" fn rs_ctrl_os_last_error(buf: *mut c_char, buf_len: size_t) -> c_int {
    if buf.is_null() || buf_len == 0 {
        return RCOS_ERR_INVALID;
    }
    let msg = LAST_ERROR.lock().map(|g| g.clone()).unwrap_or_default();
    let cap = buf_len.saturating_sub(1);
    let slice = msg.as_bytes();
    let n = slice.len().min(cap);
    unsafe {
        ptr::copy_nonoverlapping(slice.as_ptr(), buf as *mut u8, n);
        *buf.add(n) = 0;
    }
    if slice.len() >= cap && !slice.is_empty() {
        RCOS_ERR_TRUNC
    } else {
        n as c_int
    }
}

/// Free a string returned by `rs_ctrl_os_*` getters that allocate (see header).
#[no_mangle]
pub unsafe extern "C" fn rs_ctrl_os_str_free(p: *mut c_char) {
    if !p.is_null() {
        drop(CString::from_raw(p));
    }
}

// --- time sync ---

#[no_mangle]
pub extern "C" fn rs_ctrl_os_time_sync_new() -> *mut RcOsTimeSyncHandle {
    clear_last_error();
    match catch_unwind(AssertUnwindSafe(|| {
        Box::into_raw(Box::new(RcOsTimeSyncHandle {
            inner: Arc::new(TimeSynchronizer::new()),
        }))
    })) {
        Ok(p) => p,
        Err(_) => {
            set_last_error("panic in rs_ctrl_os_time_sync_new");
            ptr::null_mut()
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rs_ctrl_os_time_sync_destroy(p: *mut RcOsTimeSyncHandle) {
    if !p.is_null() {
        drop(Box::from_raw(p));
    }
}

#[no_mangle]
pub extern "C" fn rs_ctrl_os_time_sync_now_ms(p: *const RcOsTimeSyncHandle) -> u64 {
    if p.is_null() {
        return 0;
    }
    unsafe { (*p).inner.now_corrected_ms() }
}

#[no_mangle]
pub extern "C" fn rs_ctrl_os_time_sync_is_synced(p: *const RcOsTimeSyncHandle) -> c_int {
    if p.is_null() {
        return 0;
    }
    unsafe {
        if (*p).inner.is_synced() {
            1
        } else {
            0
        }
    }
}

// --- config ---

unsafe fn cstr_path<'a>(p: *const c_char) -> Result<&'a Path, c_int> {
    if p.is_null() {
        return Err(RCOS_ERR_INVALID);
    }
    let s = CStr::from_ptr(p).to_str().map_err(|_| RCOS_ERR_UTF8)?;
    Ok(Path::new(s))
}

#[no_mangle]
pub unsafe extern "C" fn rs_ctrl_os_config_open(path: *const c_char) -> *mut RcOsConfig {
    clear_last_error();
    let path = match cstr_path(path) {
        Ok(p) => p,
        Err(_) => {
            set_last_error("invalid path");
            return ptr::null_mut();
        }
    };
    match catch_unwind(AssertUnwindSafe(|| ConfigManager::<toml::Value>::new(path))) {
        Ok(Ok(mgr)) => Box::into_raw(Box::new(mgr)),
        Ok(Err(e)) => {
            set_last_error(e.to_string());
            ptr::null_mut()
        }
        Err(_) => {
            set_last_error("panic in rs_ctrl_os_config_open");
            ptr::null_mut()
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rs_ctrl_os_config_destroy(p: *mut RcOsConfig) {
    if !p.is_null() {
        drop(Box::from_raw(p));
    }
}

/// Returns the current `[dynamic]` table as **TOML text** (key/value lines; no `[dynamic]` header).
/// Hot reload is handled inside `ConfigManager` when `dynamic_load_enable` is true; this only
/// serializes the in-memory snapshot. Caller must `rs_ctrl_os_str_free`.
#[no_mangle]
pub unsafe extern "C" fn rs_ctrl_os_config_get_dynamic_toml(
    cfg: *const RcOsConfig,
    out_toml: *mut *mut c_char,
) -> c_int {
    if cfg.is_null() || out_toml.is_null() {
        return RCOS_ERR_INVALID;
    }
    clear_last_error();
    match catch_unwind(AssertUnwindSafe(|| {
        let v = (*cfg).get_dynamic_clone();
        let s = toml::to_string(&v).map_err(|e| RsCtrlError::Serialization(e.to_string()))?;
        let c = CString::new(s).map_err(|_| RsCtrlError::Serialization("NUL in TOML".into()))?;
        *out_toml = c.into_raw();
        Ok::<(), RsCtrlError>(())
    })) {
        Ok(Ok(())) => RCOS_OK,
        Ok(Err(e)) => map_err(e),
        Err(_) => {
            set_last_error("panic in rs_ctrl_os_config_get_dynamic_toml");
            RCOS_ERR_INTERNAL
        }
    }
}

macro_rules! static_str_getter {
    ($fn:ident, $field:ident) => {
        #[no_mangle]
        pub unsafe extern "C" fn $fn(cfg: *const RcOsConfig) -> *mut c_char {
            if cfg.is_null() {
                return ptr::null_mut();
            }
            clear_last_error();
            match catch_unwind(AssertUnwindSafe(|| {
                let s = (*cfg).static_cfg().$field.clone();
                CString::new(s)
                    .map(|c| c.into_raw())
                    .unwrap_or(ptr::null_mut())
            })) {
                Ok(p) => p,
                Err(_) => {
                    set_last_error(concat!(stringify!($fn), " panic"));
                    ptr::null_mut()
                }
            }
        }
    };
}

static_str_getter!(rs_ctrl_os_config_get_my_id, my_id);
static_str_getter!(rs_ctrl_os_config_get_host, host);

#[no_mangle]
pub unsafe extern "C" fn rs_ctrl_os_config_get_port(cfg: *const RcOsConfig) -> u16 {
    if cfg.is_null() {
        return 0;
    }
    (*cfg).static_cfg().port
}

#[no_mangle]
pub unsafe extern "C" fn rs_ctrl_os_config_get_is_master(cfg: *const RcOsConfig) -> c_int {
    if cfg.is_null() {
        return 0;
    }
    if (*cfg).static_cfg().is_master {
        1
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn rs_ctrl_os_config_get_publish_hz(cfg: *const RcOsConfig) -> i64 {
    if cfg.is_null() {
        return 0;
    }
    (*cfg).static_cfg().publish_hz
}

#[no_mangle]
pub unsafe extern "C" fn rs_ctrl_os_config_get_subscribe_hz(cfg: *const RcOsConfig) -> i64 {
    if cfg.is_null() {
        return 0;
    }
    (*cfg).static_cfg().subscribe_hz
}

#[no_mangle]
pub unsafe extern "C" fn rs_ctrl_os_config_get_dynamic_load_enable(
    cfg: *const RcOsConfig,
) -> c_int {
    if cfg.is_null() {
        return 0;
    }
    if (*cfg).static_cfg().dynamic_load_enable {
        1
    } else {
        0
    }
}

// --- discovery ---

/// Starts discovery. `time_sync` may be null. Returns registry handle (must pass to `rs_ctrl_os_pubsub_new`, which consumes it).
#[no_mangle]
pub unsafe extern "C" fn rs_ctrl_os_discovery_start(
    my_id: *const c_char,
    my_host: *const c_char,
    my_port: u16,
    is_master: c_int,
    time_sync: *const RcOsTimeSyncHandle,
) -> *mut ServiceRegistry {
    clear_last_error();
    let my_id = match CStr::from_ptr(my_id).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => {
            set_last_error("my_id utf-8");
            return ptr::null_mut();
        }
    };
    let my_host = match CStr::from_ptr(my_host).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => {
            set_last_error("my_host utf-8");
            return ptr::null_mut();
        }
    };
    let ts = if time_sync.is_null() {
        None
    } else {
        Some((*time_sync).inner.clone())
    };
    match catch_unwind(AssertUnwindSafe(|| {
        start_discovery(&my_id, &my_host, my_port, is_master != 0, ts)
    })) {
        Ok(Ok(reg)) => Box::into_raw(Box::new(reg)),
        Ok(Err(e)) => {
            set_last_error(e.to_string());
            ptr::null_mut()
        }
        Err(_) => {
            set_last_error("panic in rs_ctrl_os_discovery_start");
            ptr::null_mut()
        }
    }
}

/// Destroy registry only if discovery failed or you did not pass it to pubsub_new.
#[no_mangle]
pub unsafe extern "C" fn rs_ctrl_os_registry_destroy(p: *mut ServiceRegistry) {
    if !p.is_null() {
        drop(Box::from_raw(p));
    }
}

// --- pubsub ---

#[no_mangle]
pub unsafe extern "C" fn rs_ctrl_os_pubsub_new(
    cfg: *const RcOsConfig,
    registry: *mut ServiceRegistry,
) -> *mut PubSubManager {
    if cfg.is_null() || registry.is_null() {
        set_last_error("null cfg or registry");
        return ptr::null_mut();
    }
    clear_last_error();
    // Clone the Arc-based registry before consuming the raw pointer, so the
    // caller's pointer stays valid when PubSubManager::new fails.
    let clone = (*registry).clone();
    match catch_unwind(AssertUnwindSafe(|| {
        let static_cfg = (*cfg).static_cfg();
        PubSubManager::new(static_cfg, clone)
    })) {
        Ok(Ok(bus)) => {
            // Success — free the caller's registry handle (now owned by PubSubManager).
            drop(Box::from_raw(registry));
            Box::into_raw(Box::new(bus))
        }
        Ok(Err(e)) => {
            // Failure — caller retains ownership of `registry`.
            set_last_error(e.to_string());
            ptr::null_mut()
        }
        Err(_) => {
            set_last_error("panic in rs_ctrl_os_pubsub_new");
            ptr::null_mut()
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rs_ctrl_os_pubsub_destroy(p: *mut PubSubManager) {
    if !p.is_null() {
        drop(Box::from_raw(p));
    }
}

#[no_mangle]
pub unsafe extern "C" fn rs_ctrl_os_pubsub_set_publish_hz(bus: *mut PubSubManager, hz: i64) {
    if bus.is_null() {
        return;
    }
    (*bus).set_publish_hz(hz);
}

#[no_mangle]
pub unsafe extern "C" fn rs_ctrl_os_pubsub_set_subscribe_hz(bus: *mut PubSubManager, hz: i64) {
    if bus.is_null() {
        return;
    }
    (*bus).set_subscribe_hz(hz);
}

#[no_mangle]
pub unsafe extern "C" fn rs_ctrl_os_pubsub_publish_raw(
    bus: *mut PubSubManager,
    topic_key: *const c_char,
    sub_topic: *const c_char,
    payload: *const u8,
    payload_len: size_t,
) -> c_int {
    if bus.is_null() || topic_key.is_null() || sub_topic.is_null() {
        return RCOS_ERR_INVALID;
    }
    if payload.is_null() && payload_len > 0 {
        return RCOS_ERR_INVALID;
    }
    let topic_key = match CStr::from_ptr(topic_key).to_str() {
        Ok(s) => s,
        Err(_) => return RCOS_ERR_UTF8,
    };
    let sub_topic = match CStr::from_ptr(sub_topic).to_str() {
        Ok(s) => s,
        Err(_) => return RCOS_ERR_UTF8,
    };
    let pl = std::slice::from_raw_parts(payload, payload_len);
    ffi_guard_void(|| (*bus).publish_raw(topic_key, sub_topic, pl))
}

/// Non-blocking receive. On success with a message (`*got_message_out == 1`), allocates
/// `*sub_topic_out` (NUL-terminated, free with [`rs_ctrl_os_str_free`]) and `*payload_out`
/// (free with [`rs_ctrl_os_payload_free`]); `*payload_len_out` is set.
#[no_mangle]
pub unsafe extern "C" fn rs_ctrl_os_pubsub_try_recv_raw(
    bus: *mut PubSubManager,
    local_name: *const c_char,
    sub_topic_out: *mut *mut c_char,
    payload_out: *mut *mut u8,
    payload_len_out: *mut size_t,
    got_message_out: *mut c_int,
) -> c_int {
    if bus.is_null()
        || local_name.is_null()
        || sub_topic_out.is_null()
        || payload_out.is_null()
        || payload_len_out.is_null()
        || got_message_out.is_null()
    {
        return RCOS_ERR_INVALID;
    }
    *got_message_out = 0;
    *sub_topic_out = ptr::null_mut();
    *payload_out = ptr::null_mut();
    *payload_len_out = 0;
    let local_name = match CStr::from_ptr(local_name).to_str() {
        Ok(s) => s,
        Err(_) => return RCOS_ERR_UTF8,
    };
    clear_last_error();
    let res = catch_unwind(AssertUnwindSafe(|| (*bus).try_recv_raw(local_name)));
    let opt = match res {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            set_last_error(e.to_string());
            return map_err(e);
        }
        Err(_) => {
            set_last_error("panic in try_recv_raw");
            return RCOS_ERR_INTERNAL;
        }
    };
    let Some((sub_topic, payload)) = opt else {
        return RCOS_OK;
    };
    *got_message_out = 1;
    let topic_c = match CString::new(sub_topic) {
        Ok(c) => c.into_raw(),
        Err(_) => {
            set_last_error("sub_topic contains NUL");
            return RCOS_ERR_SERIALIZATION;
        }
    };
    let pptr = if payload.is_empty() {
        ptr::null_mut()
    } else {
        let layout = match std::alloc::Layout::from_size_align(payload.len(), 1) {
            Ok(l) => l,
            Err(_) => {
                drop(CString::from_raw(topic_c));
                set_last_error("invalid payload layout");
                return RCOS_ERR_INTERNAL;
            }
        };
        let raw = std::alloc::alloc(layout);
        if raw.is_null() {
            drop(CString::from_raw(topic_c));
            set_last_error("alloc payload failed");
            return RCOS_ERR_INTERNAL;
        }
        ptr::copy_nonoverlapping(payload.as_ptr(), raw, payload.len());
        raw
    };
    *sub_topic_out = topic_c;
    *payload_out = pptr;
    *payload_len_out = payload.len();
    RCOS_OK
}

/// Frees buffer returned in `payload_out` from [`rs_ctrl_os_pubsub_try_recv_raw`].
#[no_mangle]
pub unsafe extern "C" fn rs_ctrl_os_payload_free(p: *mut u8, len: size_t) {
    if p.is_null() || len == 0 {
        return;
    }
    if let Ok(layout) = std::alloc::Layout::from_size_align(len, 1) {
        std::alloc::dealloc(p, layout);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rs_ctrl_os_pubsub_set_sub_topics(
    bus: *mut PubSubManager,
    local_name: *const c_char,
    topics: *const *const c_char,
    topic_count: size_t,
) -> c_int {
    if bus.is_null() || local_name.is_null() {
        return RCOS_ERR_INVALID;
    }
    let local_name = match CStr::from_ptr(local_name).to_str() {
        Ok(s) => s,
        Err(_) => return RCOS_ERR_UTF8,
    };
    let mut list: Vec<String> = Vec::new();
    if !topics.is_null() && topic_count > 0 {
        for i in 0..topic_count {
            let p = *topics.add(i);
            if p.is_null() {
                return RCOS_ERR_INVALID;
            }
            let s = match CStr::from_ptr(p).to_str() {
                Ok(s) => s.to_string(),
                Err(_) => return RCOS_ERR_UTF8,
            };
            list.push(s);
        }
    }
    ffi_guard_void(|| (*bus).set_sub_topics(local_name, list.as_slice()))
}
