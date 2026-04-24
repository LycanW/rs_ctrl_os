pub mod comms;
pub mod config;
pub mod discovery;
pub mod error;
pub mod ffi;
pub mod time_sync;

pub use comms::PubSubManager;
pub use config::{load_config_rcos, load_config_typed, ConfigManager, StaticBase};
pub use discovery::{start_discovery, ServiceRegistry};
pub use error::{Result, RsCtrlError};
pub use time_sync::TimeSynchronizer;

pub fn init_logging() {
    use std::sync::Once;
    use tracing_subscriber::{filter::LevelFilter, fmt, prelude::*};

    static INIT: Once = Once::new();
    INIT.call_once(|| {
        tracing_subscriber::registry()
            .with(fmt::layer().with_filter(LevelFilter::INFO))
            .init();
    });
}
