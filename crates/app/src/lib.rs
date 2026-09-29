//! What the HTTP API and the chat bot share: the configuration, logging,
//! the application services and the [`App`] state that bundles them.

mod app;
pub mod config;
pub mod logging;
pub mod services;

pub use app::App;

/// Changes when the process is shutting down.
pub type ShutdownRx = tokio::sync::watch::Receiver<()>;

/// A command from the admin API to the chat bot.
#[derive(Debug)]
pub enum BotMessage {
    JoinChannels(Vec<String>),
    PartChannels(Vec<String>),
}
