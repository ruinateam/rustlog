/// Why a request for opted-out data is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OptedOut {
    #[error("the requested channel has opted out of being logged")]
    Channel,
    #[error("the requested user has opted out of being logged")]
    User,
}
