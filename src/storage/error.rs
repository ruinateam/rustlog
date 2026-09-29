/// Failure of a storage operation.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]
    Database(Box<clickhouse::error::Error>),
    /// The query matched no messages.
    #[error("not found")]
    NotFound,
}

impl From<clickhouse::error::Error> for Error {
    fn from(error: clickhouse::error::Error) -> Self {
        Self::Database(Box::new(error))
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
