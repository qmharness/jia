#[derive(Debug, thiserror::Error)]
pub enum JiaError {
    #[error("config: {0}")]
    Config(String),
    #[error("database: {0}")]
    Database(#[from] r2d2::Error),
    #[error("network: {0}")]
    Network(String),
    #[error("internal: {0}")]
    Internal(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
