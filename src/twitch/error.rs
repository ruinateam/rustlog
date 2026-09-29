use twitch_api::helix::ClientRequestError;

/// Failure of a Twitch API request.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The app access token has not been obtained yet.
    #[error("Twitch token is not ready yet")]
    TokenUnavailable,
    #[error(transparent)]
    Helix(Box<ClientRequestError<reqwest::Error>>),
    /// Twitch does not know the user.
    #[error("not found")]
    NotFound,
}

impl From<ClientRequestError<reqwest::Error>> for Error {
    fn from(error: ClientRequestError<reqwest::Error>) -> Self {
        Self::Helix(Box::new(error))
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
