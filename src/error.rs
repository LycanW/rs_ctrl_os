use thiserror::Error;

#[derive(Error, Debug)]
pub enum RsCtrlError {
    #[error("Config error: {0}")]
    Config(String),
    #[error("Communication error: {0}")]
    Comms(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Discovery error: {0}")]
    Discovery(String),
    #[error("Node '{0}' not found in registry")]
    NodeNotFound(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("ZMQ error: {0}")]
    Zmq(#[from] zmq::Error),
    #[error("Bincode error: {0}")]
    Bincode(#[from] Box<bincode::ErrorKind>),
}

pub type Result<T> = std::result::Result<T, RsCtrlError>;
