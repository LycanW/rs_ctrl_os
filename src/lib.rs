pub mod error;
pub mod config;
pub mod discovery;
pub mod comms;
pub mod time_sync;

pub use error::{Result, RsCtrlError};
pub use config::{load_config_rcos, load_config_typed, ConfigManager, StaticBase};
pub use discovery::{start_discovery, ServiceRegistry};
pub use comms::PubSubManager;
pub use time_sync::TimeSynchronizer;

pub fn init_logging() {
    use tracing_subscriber::{fmt, prelude::*, filter::LevelFilter};

    tracing_subscriber::registry()
        .with(fmt::layer().with_filter(LevelFilter::INFO))
        .init();
}