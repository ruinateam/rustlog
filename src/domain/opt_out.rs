/// Why a request for opted-out data is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OptedOut {
    #[error("The requested channel has opted out of being logged")]
    Channel,
    #[error("The requested user has opted out of being logged")]
    User,
}
