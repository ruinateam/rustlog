use crate::{
    config::Config,
    domain::opt_out::OptedOut,
    services::{sully::SullyGnome, tiers::Tiers},
    state::{OperationalState, OptOutScope},
    storage::{logs::delete_user_logs, message::StructuredMessage, writer::FlushBuffer},
    twitch::Twitch,
};
use anyhow::Context;
use dashmap::DashSet;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::info;

/// A command from the admin API to the chat bot.
#[derive(Debug)]
pub enum BotMessage {
    JoinChannels(Vec<String>),
    PartChannels(Vec<String>),
}

/// Everything the HTTP handlers and the bot share.
#[derive(Clone)]
pub struct App {
    pub twitch: Twitch,
    pub sully: SullyGnome,
    pub tiers: Tiers,
    pub optout_codes: Arc<DashSet<String>>,
    pub db: Arc<clickhouse::Client>,
    pub config: Arc<Config>,
    pub state: OperationalState,
    pub flush_buffer: FlushBuffer,
    pub firehose_tx: broadcast::Sender<StructuredMessage<'static>>,
}

impl App {
    /// Stops logging the user and deletes their messages and logins.
    pub async fn opt_out_user(&self, user_id: &str) -> anyhow::Result<()> {
        self.state
            .set_opted_out(OptOutScope::User, user_id, true)
            .await?;
        self.flush_buffer.remove_user(user_id).await;
        delete_user_logs(&self.db, user_id)
            .await
            .context("could not delete logs")?;

        info!(user_id, "user opted out");
        Ok(())
    }

    /// Logs the user again from now on; deleted messages stay deleted.
    pub async fn opt_in_user(&self, user_id: &str) -> anyhow::Result<()> {
        self.state
            .set_opted_out(OptOutScope::User, user_id, false)
            .await?;
        info!(user_id, "user opted back in");
        Ok(())
    }

    /// Stops logging the channel and hides its logs; nothing is deleted.
    /// The bot stays in the chat, so that the channel can opt back in there.
    pub async fn opt_out_channel(&self, channel_id: &str) -> anyhow::Result<()> {
        self.state
            .set_opted_out(OptOutScope::Channel, channel_id, true)
            .await?;
        info!(channel_id, "channel opted out");
        Ok(())
    }

    /// Logs the channel again and shows its logs, including the old ones.
    pub async fn opt_in_channel(&self, channel_id: &str) -> anyhow::Result<()> {
        self.state
            .set_opted_out(OptOutScope::Channel, channel_id, false)
            .await?;
        info!(channel_id, "channel opted back in");
        Ok(())
    }

    pub fn check_opted_out(&self, channel_id: &str, user_id: Option<&str>) -> Result<(), OptedOut> {
        if self.state.is_channel_opted_out(channel_id) {
            return Err(OptedOut::Channel);
        }

        if let Some(user_id) = user_id
            && self.state.is_user_opted_out(user_id)
        {
            return Err(OptedOut::User);
        }

        Ok(())
    }

    pub fn check_user_opted_out(&self, user_id: &str) -> Result<(), OptedOut> {
        if self.state.is_user_opted_out(user_id) {
            return Err(OptedOut::User);
        }

        Ok(())
    }
}
