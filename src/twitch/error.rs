use twitch_api::helix::ClientRequestError;

/// A failed Helix request.
pub type HelixError = ClientRequestError<reqwest::Error>;

/// Failure of a Twitch API request.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The app access token has not been obtained yet.
    #[error("Twitch token is not ready yet")]
    TokenUnavailable,
    #[error(transparent)]
    Helix(Box<HelixError>),
    /// Twitch does not know the user.
    #[error("not found")]
    NotFound,
}

impl From<HelixError> for Error {
    fn from(error: HelixError) -> Self {
        Self::Helix(Box::new(error))
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
